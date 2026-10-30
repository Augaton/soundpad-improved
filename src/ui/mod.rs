// The window: search and actions on top, categories on the left, the sounds in the middle,
// what is playing and the volumes on the right.
mod capture;
mod dialogs;
mod theme;
mod tray;
mod widgets;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui::{
    self, Align, Align2, CentralPanel, ComboBox, Context, CursorIcon, Frame, Id, Key,
    KeyboardShortcut, Label, Layout, Margin, Modifiers, Panel, Rect, RichText, ScrollArea, Sense,
    Stroke, StrokeKind, TextEdit, Ui, ViewportCommand, pos2, vec2,
};

use crate::audio::engine::Status;
use crate::audio::mixer::{CABLE, MONITOR, VoiceInfo};
use crate::keys;
use crate::model::{Cable, Category, Library, Sound, reorder, search_key};
use crate::state::{Core, Snapshot, Toast, lock};
use crate::store;
use capture::Capture;
use dialogs::{CategoryHotkey, Confirm, Editor, LinkDialog};
use theme::*;
pub use tray::wake_running;
use widgets::*;

pub fn window_icon() -> egui::IconData {
    egui::IconData {
        rgba: app_icon(64),
        width: 64,
        height: 64,
    }
}

#[derive(Clone, PartialEq)]
enum View {
    All,
    Favorites,
    Category(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Manual, // the order of the library, or of the category; sounds are dragged to change it
    Title,
    Plays,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Sort::Manual => "Mon ordre",
            Sort::Title => "Titre",
            Sort::Plays => "Les plus joués",
        }
    }
}

/// A sound being dragged to a new place in the grid.
struct Drag {
    id: String,
    title: String,
    target: Option<(String, bool)>, // the sound it goes next to, after it (true) or before
}

pub struct App {
    core: Arc<Core>,
    view: View,
    query: String,
    sort: Sort,
    new_category: String,
    editor: Option<Editor>,
    settings: bool,
    settings_error: Option<String>,
    autostart: bool, // read from the registry when the settings open
    confirm: Option<Confirm>,
    link: Option<LinkDialog>,
    category_hotkey: Option<CategoryHotkey>,
    capture: Option<Capture>,
    toasts: Vec<Toast>,
    meter: f32,
    seeking: HashMap<u64, f32>, // voice id -> position (0..1) while its bar is dragged
    warm: HashSet<String>,      // sounds already decoded ahead on hover
    hover: Option<(String, f64)>, // the sound under the pointer, since when (egui time)
    selected: HashSet<String>,  // sound ids, picked with Ctrl or Shift + click
    anchor: Option<String>,     // where a Shift + click range starts
    drag: Option<Drag>,
    focus_search: bool,
    snap: Arc<Snapshot>, // the library as drawn, taken again only when it changed
    rev: u64,
    visible: Vec<usize>, // indices of the sounds shown, for (revision, view, query, sort)
    visible_for: Option<(u64, View, String, Sort)>,
    tray: Option<tray::Tray>,
    quitting: Arc<AtomicBool>, // "Quitter" in the icon's menu: the window closes for real
    hide_at_start: bool,       // launched with Windows: straight to the notification area
    // In the notification area. eframe still runs the window's frames then (winit reports no
    // occlusion on Windows): nothing is built or drawn.
    hidden: Arc<AtomicBool>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, core: Arc<Core>, start_hidden: bool) -> App {
        theme::install(&cc.egui_ctx);
        theme::dark_title_bar(cc);
        core.start(cc.egui_ctx.clone());
        let snap = Arc::new(core.snapshot());
        let quitting = Arc::new(AtomicBool::new(false));
        let hidden = Arc::new(AtomicBool::new(false));
        let tray = {
            let (ctx, core) = (cc.egui_ctx.clone(), core.clone());
            let (quitting, hidden) = (quitting.clone(), hidden.clone());
            tray::start(move |command| match command {
                tray::Command::Show => {
                    hidden.store(false, Ordering::Relaxed);
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                    ctx.request_repaint();
                }
                tray::Command::StopAll => core.engine.stop_all(),
                tray::Command::Quit => {
                    quitting.store(true, Ordering::Relaxed);
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    ctx.request_repaint();
                }
            })
        };
        App {
            core,
            view: View::All,
            query: String::new(),
            sort: Sort::Manual,
            new_category: String::new(),
            editor: None,
            settings: false,
            settings_error: None,
            autostart: false,
            confirm: None,
            link: None,
            category_hotkey: None,
            capture: None,
            toasts: Vec::new(),
            meter: 0.0,
            seeking: HashMap::new(),
            warm: HashSet::new(),
            hover: None,
            selected: HashSet::new(),
            anchor: None,
            drag: None,
            focus_search: false,
            snap,
            rev: u64::MAX,
            visible: Vec::new(),
            visible_for: None,
            hide_at_start: start_hidden && tray.is_some(),
            tray,
            quitting,
            hidden,
        }
    }

    fn hide(&self, ctx: &Context) {
        self.hidden.store(true, Ordering::Relaxed);
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    fn set_view(&mut self, view: View) {
        if self.view != view {
            self.selected.clear();
            self.anchor = None;
        }
        self.view = view;
    }

    /// No dialog open, no text being typed: the window's own shortcuts may act.
    fn idle(&self, ctx: &Context) -> bool {
        self.editor.is_none()
            && !self.settings
            && self.confirm.is_none()
            && self.link.is_none()
            && self.category_hotkey.is_none()
            && self.capture.is_none()
            && ctx.memory(|m| m.focused().is_none())
    }

    /// The sounds shown, filtered and sorted again only when the library or the view changed.
    fn refresh_visible(&mut self, lib: &Library) {
        let key = (self.rev, self.view.clone(), self.query.clone(), self.sort);
        if self.visible_for.as_ref() == Some(&key) {
            return;
        }
        let sounds = &lib.sounds;
        let mut list: Vec<usize> = match &self.view {
            View::All => (0..sounds.len()).collect(),
            View::Favorites => (0..sounds.len()).filter(|&i| sounds[i].favorite).collect(),
            View::Category(id) => {
                let index: HashMap<&str, usize> = sounds
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (s.id.as_str(), i))
                    .collect();
                let ids = lib
                    .categories
                    .iter()
                    .find(|c| c.id == *id)
                    .map(|c| c.sound_ids.as_slice())
                    .unwrap_or_default();
                ids.iter()
                    .filter_map(|id| index.get(id.as_str()).copied())
                    .collect()
            }
        };
        let q = search_key(self.query.trim());
        if !q.is_empty() {
            list.retain(|&i| search_key(&sounds[i].title).contains(&q));
        }
        match self.sort {
            Sort::Manual => {}
            Sort::Title => list
                .sort_by_cached_key(|&i| (search_key(&sounds[i].title), sounds[i].title.clone())),
            Sort::Plays => list.sort_by_key(|&i| std::cmp::Reverse(sounds[i].plays)),
        }
        self.visible = list;
        self.visible_for = Some(key);
    }

    fn view_category(&self) -> Option<String> {
        match &self.view {
            View::Category(id) => Some(id.clone()),
            _ => None,
        }
    }

    // ---------- Top bar ----------

    fn top_bar(&mut self, ui: &mut Ui) {
        Panel::top("top")
            .exact_size(56.0)
            .show_separator_line(false)
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(16, 0)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    Frame::new()
                        .fill(theme::SURFACE)
                        .stroke(Stroke::new(1.0, LINE))
                        .corner_radius(RADIUS)
                        .inner_margin(Margin::symmetric(10, 0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(I_SEARCH).font(icon_font(13.0)).color(TEXT_3),
                                );
                                let edit = ui.add(
                                    TextEdit::singleline(&mut self.query)
                                        .hint_text("Rechercher un son   Ctrl+F")
                                        .frame(Frame::NONE)
                                        .desired_width(280.0),
                                );
                                if std::mem::take(&mut self.focus_search) {
                                    edit.request_focus();
                                }
                                if edit.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                                    self.query.clear();
                                }
                                let clear = ui.add_visible(
                                    !self.query.is_empty(),
                                    egui::Button::new(
                                        RichText::new(I_CLOSE).font(icon_font(10.0)).color(TEXT_3),
                                    )
                                    .frame(false),
                                );
                                if clear.on_hover_text("Effacer la recherche").clicked() {
                                    self.query.clear();
                                }
                            });
                        });
                    ComboBox::from_id_salt("sort")
                        .selected_text(format!("Tri : {}", self.sort.label()))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for s in [Sort::Manual, Sort::Title, Sort::Plays] {
                                ui.selectable_value(&mut self.sort, s, s.label());
                            }
                        })
                        .response
                        .on_hover_text("« Mon ordre » : glisse un son pour le déplacer");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if danger(ui, "Tout arrêter")
                            .on_hover_text(self.global_hint(crate::model::GlobalAction::StopAll))
                            .clicked()
                        {
                            self.core.engine.stop_all();
                        }
                        if icon_button(ui, I_SETTINGS, "Réglages", self.settings).clicked() {
                            self.open_settings();
                        }
                        let link_hint =
                            "Ajouter depuis un lien (Voicemod Tuna, MyInstants…)   Ctrl+V";
                        if icon_button(ui, I_LINK, link_hint, self.link.is_some()).clicked() {
                            self.link = Some(LinkDialog::new(""));
                        }
                        if icon_text(ui, I_ADD, "Ajouter des sons").clicked() {
                            self.pick_and_add();
                        }
                        let update = lock(&self.core.update).clone();
                        if let Some(v) = update
                            && icon_text(ui, I_UPDATE, &format!("Version {v} disponible"))
                                .on_hover_text("Ouvre la page de téléchargement")
                                .clicked()
                        {
                            crate::open_url(crate::state::RELEASES);
                        }
                    });
                });
            });
    }

    fn global_hint(&self, a: crate::model::GlobalAction) -> String {
        match self.core.read(|l| l.settings.hotkeys.get(a).clone()) {
            Some(k) => format!("Raccourci global : {}", keys::format_accelerator(&k)),
            None => "Un raccourci global se règle dans les réglages".into(),
        }
    }

    fn pick_and_add(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Ajouter des sons")
            .add_filter("Audio", &store::AUDIO_EXT)
            .pick_files()
            .unwrap_or_default();
        let paths: Vec<String> = files
            .iter()
            .filter(|p| store::is_audio_path(p))
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        self.core.add_paths(&paths, self.view_category().as_deref());
    }

    pub fn import_soundpad(&mut self) {
        let mut file = store::soundpad_file();
        if !file.exists() {
            let picked = rfd::FileDialog::new()
                .set_title("Liste de sons Soundpad")
                .add_filter("Liste Soundpad", &["spl"])
                .pick_file();
            match picked {
                Some(p) => file = p,
                None => return,
            }
        }
        self.core.import_soundpad(&file);
    }

    fn open_settings(&mut self) {
        self.settings = true;
        self.settings_error = None;
        self.autostart = crate::autostart::enabled();
        if self.core.read(|l| l.settings.cable_device == Cable::Filter) {
            let core = self.core.clone();
            std::thread::spawn(move || core.refresh_filter());
        }
    }

    // ---------- Banner ----------

    fn banner(&mut self, ui: &mut Ui, lib: &Library, status: &Status) {
        let cable_found = status
            .outputs
            .iter()
            .any(|n| crate::model::is_virtual_cable(n));
        let filter = lib.settings.cable_device == Cable::Filter;
        let text = if filter {
            let f = lock(&self.core.filter);
            match &f.status {
                Some(s) if s.ok && s.installed.is_empty() => {
                    "Le filtre micro n’est pas installé : les sons ne sont entendus que par toi. Installe-le dans les réglages."
                }
                _ => "",
            }
        } else if !status.running[CABLE] && !status.outputs.is_empty() {
            if cable_found {
                "L’envoi aux autres est désactivé : les sons ne sont entendus que par toi."
            } else {
                "Les sons ne sont entendus que par toi. Dans les réglages, choisis « Mon micro, avec le filtre » (comme Soundpad), ou installe le câble virtuel VB-Cable."
            }
        } else {
            ""
        };
        if text.is_empty() {
            return;
        }
        Panel::top("banner")
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(Margin::symmetric(16, 8))
                    .stroke(Stroke::new(1.0, LINE)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(I_WARNING).font(icon_font(14.0)).color(ACCENT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Réglages").clicked() {
                            self.open_settings();
                        }
                        if !cable_found && !filter && ui.button("Télécharger VB-Cable").clicked()
                        {
                            crate::open_url("https://vb-audio.com/Cable/");
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(Label::new(RichText::new(text).color(TEXT_2)).wrap())
                        });
                    });
                });
            });
    }

    // ---------- Categories ----------

    fn sidebar(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let lib = &snap.lib;
        ui.add_space(4.0);
        let favorites = lib.sounds.iter().filter(|s| s.favorite).count();
        self.category_row(
            ui,
            View::All,
            "Tous les sons",
            lib.sounds.len(),
            None,
            false,
        );
        self.category_row(ui, View::Favorites, "Favoris", favorites, None, false);
        ui.add_space(10.0);
        ui.label(RichText::new("Catégories").color(TEXT_3).size(12.0));
        ui.add_space(2.0);
        ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(ui.available_height() - 48.0)
            .show(ui, |ui| {
                for c in &lib.categories {
                    let failed = c
                        .hotkey
                        .as_ref()
                        .is_some_and(|h| snap.failed_hotkeys.contains(h));
                    self.category_row(
                        ui,
                        View::Category(c.id.clone()),
                        &c.name,
                        c.sound_ids.len(),
                        Some(c),
                        failed,
                    );
                }
            });
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let edit = ui.add(
                    TextEdit::singleline(&mut self.new_category)
                        .hint_text("Nouvelle catégorie")
                        .char_limit(60)
                        .desired_width(ui.available_width() - 36.0),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if icon_button(ui, I_ADD, "Créer la catégorie", false).clicked() || enter {
                    self.create_category();
                }
            });
        });
    }

    fn category_row(
        &mut self,
        ui: &mut Ui,
        view: View,
        name: &str,
        count: usize,
        category: Option<&Category>,
        hotkey_failed: bool,
    ) {
        let selected = self.view == view;
        let (rect, mut resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
        if let Some(c) = category {
            resp = resp.on_hover_text("Clic droit : raccourci qui joue un de ses sons au hasard");
            resp.context_menu(|ui| {
                if ui.button("Raccourci : un son au hasard…").clicked() {
                    self.category_hotkey = Some(CategoryHotkey::new(&c.id));
                    ui.close();
                }
            });
        }
        let remove_rect =
            Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(24.0, 24.0));
        let remove = category.map(|c| {
            ui.interact(
                remove_rect,
                Id::unique(("remove-cat", &c.id)),
                Sense::click(),
            )
            .on_hover_text("Supprimer la catégorie")
        });
        let hovered = resp.hovered() || remove.as_ref().is_some_and(|r| r.hovered());
        let p = ui.painter();
        if selected || hovered {
            p.rect_filled(rect, RADIUS, if selected { SURFACE_3 } else { SURFACE_2 });
        }
        if selected {
            p.rect_filled(
                Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(2.0, 16.0)),
                1,
                ACCENT,
            );
        }
        if resp.has_focus() {
            p.rect_stroke(rect, RADIUS, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
        }
        // From the right: the count (or the remove button), the hotkey, the name.
        let show_remove = hovered && remove.is_some();
        let mut right = if !show_remove {
            p.text(
                pos2(rect.right() - 10.0, rect.center().y),
                Align2::RIGHT_CENTER,
                count.to_string(),
                egui::FontId::proportional(12.0),
                TEXT_3,
            )
            .left()
        } else {
            let r = remove.as_ref().unwrap();
            if r.hovered() {
                p.rect_filled(remove_rect, RADIUS, HOVER);
            }
            p.text(
                remove_rect.center(),
                Align2::CENTER_CENTER,
                I_CLOSE,
                icon_font(10.0),
                if r.hovered() { DANGER } else { TEXT_2 },
            );
            remove_rect.left()
        };
        if let Some(key) = category.and_then(|c| c.hotkey.as_deref()) {
            let chip = key_chip(
                ui,
                right - 6.0,
                rect.center().y,
                &keys::format_accelerator(key),
                hotkey_failed,
            );
            right = chip.left();
        }
        let text_color = if selected { TEXT } else { TEXT_2 };
        let galley = ui.painter().layout(
            name.to_string(),
            egui::FontId::proportional(14.0),
            text_color,
            right - rect.left() - 18.0,
        );
        ui.painter().galley(
            pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
            galley,
            text_color,
        );
        if let (Some(r), Some(c)) = (&remove, category)
            && r.clicked()
        {
            self.confirm = Some(Confirm::RemoveCategory {
                id: c.id.clone(),
                name: c.name.clone(),
            });
        } else if resp.clicked() {
            self.set_view(view);
        }
    }

    fn create_category(&mut self) {
        let name = self.new_category.trim().to_string();
        if name.is_empty() {
            return;
        }
        let id = self.core.edit(|lib| {
            if let Some(c) = lib
                .categories
                .iter()
                .find(|c| c.name.to_lowercase() == name.to_lowercase())
            {
                c.id.clone()
            } else {
                let c = Category {
                    id: crate::model::new_id(),
                    name,
                    sound_ids: Vec::new(),
                    hotkey: None,
                };
                let id = c.id.clone();
                lib.categories.push(c);
                id
            }
        });
        self.core.save_now();
        self.set_view(View::Category(id));
        self.new_category.clear();
    }

    // ---------- Sounds ----------

    fn grid(&mut self, ui: &mut Ui, snap: &Snapshot, voices: &[VoiceInfo]) {
        let lib = &snap.lib;
        self.refresh_visible(lib);
        if !self.selected.is_empty() {
            self.selection_bar(ui, lib);
        }
        if self.visible.is_empty() {
            self.empty_state(ui, lib);
            return;
        }
        let list: Vec<&Sound> = self.visible.iter().map(|&i| &lib.sounds[i]).collect();
        let (missing, failed) = (&snap.missing, &snap.failed_hotkeys);
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let mut progress: HashMap<&str, f32> = HashMap::new(); // newest voice of each sound
        for v in voices {
            *counts.entry(&v.sound_id).or_default() += 1;
            let p = self
                .seeking
                .get(&v.id)
                .copied()
                .unwrap_or(if v.duration > 0.0 {
                    (v.position / v.duration) as f32
                } else {
                    0.0
                });
            progress.insert(&v.sound_id, p);
        }

        let width = ui.available_width() - 14.0; // room for the floating scroll bar
        let cols = (((width + GAP) / (176.0 + GAP)).floor() as usize).max(1);
        let pad_w = (width - GAP * (cols - 1) as f32) / cols as f32;
        let rows = list.len().div_ceil(cols);
        // Sounds move by dragging only where the grid shows their own order, all of them.
        let draggable = self.sort == Sort::Manual && self.query.trim().is_empty();
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let mut target = None;
        let mut hovered = false;
        ui.spacing_mut().item_spacing.y = GAP;
        ScrollArea::vertical()
            .auto_shrink(false)
            .show_rows(ui, PAD_HEIGHT, rows, |ui, range| {
                // Held near the top or bottom edge, a dragged sound scrolls the grid.
                if self.drag.is_some()
                    && let Some(p) = pointer
                {
                    let clip = ui.clip_rect();
                    if p.y < clip.top() + 40.0 {
                        ui.scroll_with_delta(vec2(0.0, 12.0));
                    } else if p.y > clip.bottom() - 40.0 {
                        ui.scroll_with_delta(vec2(0.0, -12.0));
                    }
                }
                for row in range {
                    let (row_rect, _) =
                        ui.allocate_exact_size(vec2(width, PAD_HEIGHT), Sense::hover());
                    for (col, sound) in list.iter().skip(row * cols).take(cols).enumerate() {
                        let rect = Rect::from_min_size(
                            row_rect.min + vec2(col as f32 * (pad_w + GAP), 0.0),
                            vec2(pad_w, PAD_HEIGHT),
                        );
                        let playing = counts.get(sound.id.as_str()).copied().unwrap_or(0);
                        let view = Pad {
                            title: &sound.title,
                            duration: sound.duration.filter(|d| *d > 0.0),
                            hotkey: sound.hotkey.as_deref().map(keys::format_accelerator),
                            hotkey_failed: sound
                                .hotkey
                                .as_ref()
                                .is_some_and(|h| failed.contains(h)),
                            favorite: sound.favorite,
                            missing: missing.contains(&sound.id),
                            selected: self.selected.contains(&sound.id),
                            playing,
                            progress: (playing > 0)
                                .then(|| progress.get(sound.id.as_str()).copied().unwrap_or(0.0)),
                        };
                        let (action, resp) =
                            pad(ui, rect, Id::unique(("pad", &sound.id)), &view, draggable);
                        if resp.hovered() && !view.missing {
                            hovered = true;
                            self.warm_up(ui.ctx(), sound);
                        }
                        if draggable && resp.drag_started() {
                            self.drag = Some(Drag {
                                id: sound.id.clone(),
                                title: sound.title.clone(),
                                target: None,
                            });
                        }
                        if let Some(drag) = &self.drag {
                            if drag.id == sound.id {
                                ui.painter()
                                    .rect_filled(rect, RADIUS, BG.gamma_multiply(0.6));
                            } else if let Some(p) = pointer
                                && rect.expand(GAP / 2.0).contains(p)
                            {
                                let after = p.x > rect.center().x;
                                let x = if after {
                                    rect.right() + GAP / 2.0
                                } else {
                                    rect.left() - GAP / 2.0
                                };
                                ui.painter()
                                    .vline(x, rect.y_range(), Stroke::new(3.0, ACCENT));
                                target = Some((sound.id.clone(), after));
                            }
                        }
                        match action {
                            PadAction::Play => {
                                let modifiers = ui.input(|i| i.modifiers);
                                self.click_pad(modifiers, &sound.id, &list);
                            }
                            PadAction::Edit => self.editor = Some(Editor::open(sound, lib)),
                            PadAction::None => {}
                        }
                    }
                }
            });
        if !hovered {
            self.hover = None;
        }
        if let Some(drag) = &mut self.drag {
            drag.target = target;
        }
        self.end_drag(ui.ctx());
    }

    /// A sound the pointer rests on is decoded ahead, so the click plays at once. Not one it
    /// only passes over: scrolling the grid would decode the whole library.
    fn warm_up(&mut self, ctx: &Context, sound: &Sound) {
        const REST: f64 = 0.15; // seconds
        let now = ctx.input(|i| i.time);
        match &self.hover {
            Some((id, since)) if *id == sound.id => {
                if now - since >= REST && self.warm.insert(sound.id.clone()) {
                    self.core.engine.preload(&sound.path);
                }
            }
            _ => {
                self.hover = Some((sound.id.clone(), now));
                ctx.request_repaint_after(Duration::from_secs_f64(REST));
            }
        }
    }

    /// A click on a sound plays it. With Ctrl, it goes into the selection or out of it; with
    /// Shift, every sound from the last one clicked to this one is selected.
    fn click_pad(&mut self, modifiers: Modifiers, id: &str, list: &[&Sound]) {
        if modifiers.command {
            if !self.selected.remove(id) {
                self.selected.insert(id.to_string());
            }
            self.anchor = Some(id.to_string());
        } else if modifiers.shift {
            let at = |x: &str| list.iter().position(|s| s.id == x);
            let end = at(id);
            let start = self.anchor.as_deref().and_then(at).or(end);
            if let (Some(a), Some(b)) = (start, end) {
                for s in &list[a.min(b)..=a.max(b)] {
                    self.selected.insert(s.id.clone());
                }
            }
            if self.anchor.is_none() {
                self.anchor = Some(id.to_string());
            }
        } else {
            self.core.trigger(id);
        }
    }

    /// The dragged sound follows the pointer; released, it takes its new place.
    fn end_drag(&mut self, ctx: &Context) {
        let Some(drag) = &self.drag else { return };
        if ctx.input(|i| i.pointer.any_down()) {
            ctx.set_cursor_icon(CursorIcon::Grabbing);
            if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                egui::Area::new(Id::unique("drag"))
                    .fixed_pos(p + vec2(14.0, 10.0))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ctx, |ui| {
                        Frame::new()
                            .fill(SURFACE_3)
                            .stroke(Stroke::new(1.0, ACCENT))
                            .corner_radius(RADIUS)
                            .inner_margin(Margin::symmetric(10, 6))
                            .show(ui, |ui| ui.label(RichText::new(&drag.title).color(TEXT)));
                    });
            }
            ctx.request_repaint(); // the edge scrolling goes on while the pointer stays still
            return;
        }
        let Some(Drag {
            id,
            target: Some((target, after)),
            ..
        }) = self.drag.take()
        else {
            return;
        };
        let view = self.view.clone();
        self.core.edit(|lib| match &view {
            View::Category(c) => {
                if let Some(c) = lib.categories.iter_mut().find(|x| x.id == *c) {
                    let at = |x: &str| c.sound_ids.iter().position(|s| s == x);
                    if let (Some(from), Some(to)) = (at(&id), at(&target)) {
                        reorder(&mut c.sound_ids, from, to, after);
                    }
                }
            }
            View::All | View::Favorites => {
                let at = |x: &str| lib.sounds.iter().position(|s| s.id == x);
                if let (Some(from), Some(to)) = (at(&id), at(&target)) {
                    reorder(&mut lib.sounds, from, to, after);
                }
            }
        });
        self.core.save();
    }

    /// What can be done to the selected sounds at once.
    /// Drawn at every frame: one pass over the library, the selection looked up in its set.
    fn selection_bar(&mut self, ui: &mut Ui, lib: &Library) {
        let (mut n, mut all_favorite) = (0, true);
        for s in lib.sounds.iter().filter(|s| self.selected.contains(&s.id)) {
            n += 1;
            all_favorite &= s.favorite;
        }
        let plural = if n > 1 { "s" } else { "" };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{n} son{plural} sélectionné{plural}"))
                    .family(strong())
                    .color(TEXT),
            );
            ui.add_space(8.0);
            ui.menu_button("Ajouter à une catégorie", |ui| {
                if lib.categories.is_empty() {
                    hint(
                        ui,
                        "Aucune catégorie : crée-en une dans la colonne de gauche.",
                    );
                }
                for c in &lib.categories {
                    if ui.button(&c.name).clicked() {
                        let category = c.id.clone();
                        let selected = &self.selected;
                        self.core.edit(|lib| {
                            let ids: Vec<String> = lib
                                .sounds
                                .iter()
                                .filter(|s| selected.contains(&s.id))
                                .map(|s| s.id.clone())
                                .collect();
                            if let Some(c) = lib.categories.iter_mut().find(|c| c.id == category) {
                                let present: HashSet<String> =
                                    c.sound_ids.iter().cloned().collect();
                                c.sound_ids
                                    .extend(ids.into_iter().filter(|id| !present.contains(id)));
                            }
                        });
                        self.core.save();
                        self.core
                            .toast(format!("Ajouté{plural} à « {} »", c.name), false);
                        ui.close();
                    }
                }
            });
            if let View::Category(category) = self.view.clone()
                && ui.button("Retirer de la catégorie").clicked()
            {
                let selected = &self.selected;
                self.core.edit(|lib| {
                    if let Some(c) = lib.categories.iter_mut().find(|c| c.id == category) {
                        c.sound_ids.retain(|id| !selected.contains(id));
                    }
                });
                self.core.save();
                self.selected.clear();
            }
            let label = if all_favorite {
                "Retirer des favoris"
            } else {
                "Mettre en favori"
            };
            if ui.button(label).clicked() {
                let selected = &self.selected;
                self.core.edit(|lib| {
                    for s in lib.sounds.iter_mut().filter(|s| selected.contains(&s.id)) {
                        s.favorite = !all_favorite;
                    }
                });
                self.core.save();
            }
            if danger(ui, "Retirer de la bibliothèque").clicked() {
                self.confirm = Some(Confirm::RemoveSounds {
                    label: format!("{n} son{plural}"),
                    ids: self.selected.iter().cloned().collect(),
                });
            }
            if ui.button("Désélectionner").on_hover_text("Échap").clicked() {
                self.selected.clear();
            }
        });
        ui.add_space(GAP);
    }

    fn empty_state(&mut self, ui: &mut Ui, lib: &Library) {
        let soundpad = *lock(&self.core.soundpad_count);
        let query = self.query.trim().to_string();
        let (title, text) = if lib.sounds.is_empty() {
            (
                "Ta bibliothèque est vide".to_string(),
                if soundpad > 0 {
                    format!(
                        "Soundpad est installé avec {soundpad} son{} : importe-les en un clic (catégories et raccourcis compris), ou glisse ici des fichiers audio ou des dossiers.",
                        if soundpad > 1 { "s" } else { "" }
                    )
                } else {
                    "Ajoute des sons, glisse ici des fichiers audio ou des dossiers, ou colle (Ctrl+V) le lien d’un son Voicemod Tuna ou MyInstants.".to_string()
                },
            )
        } else if !query.is_empty() {
            (
                "Aucun résultat".to_string(),
                format!("Aucun son ne correspond à « {query} »."),
            )
        } else if self.view == View::Favorites {
            (
                "Aucun favori".to_string(),
                "Coche « Favori » dans la fenêtre d’un son (bouton ••• ou clic droit sur le son)."
                    .to_string(),
            )
        } else {
            ("Catégorie vide".to_string(), "Glisse des fichiers sur cette vue, ou coche cette catégorie dans la fenêtre d’un son.".to_string())
        };
        ui.add_space((ui.available_height() * 0.28).max(24.0));
        ui.vertical_centered(|ui| {
            ui.set_max_width(440.0);
            ui.label(RichText::new(title).heading().color(TEXT));
            ui.add_space(4.0);
            ui.label(RichText::new(text).color(TEXT_2));
            if lib.sounds.is_empty() {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let w = if soundpad > 0 { 460.0 } else { 280.0 };
                    ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                    if soundpad > 0 && primary(ui, "Importer depuis Soundpad").clicked() {
                        self.import_soundpad();
                    }
                    if ui.button("Ajouter des sons").clicked() {
                        self.pick_and_add();
                    }
                    if ui.button("Depuis un lien").clicked() {
                        self.link = Some(LinkDialog::new(""));
                    }
                });
            }
        });
    }

    // ---------- Now playing and volumes ----------

    fn playing(&mut self, ui: &mut Ui, lib: &Library, voices: &[VoiceInfo]) {
        Panel::bottom("mixer")
            .show_separator_line(false)
            .frame(Frame::new().inner_margin(Margin {
                left: 0,
                right: 0,
                top: 12,
                bottom: 14,
            }))
            .show(ui, |ui| {
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.max_rect().top(),
                    Stroke::new(1.0, LINE),
                );
                self.mixer(ui, lib);
            });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("En cours").heading());
            if !voices.is_empty() {
                ui.label(RichText::new(voices.len().to_string()).color(TEXT_3));
            }
        });
        ui.add_space(6.0);
        if voices.is_empty() {
            ui.label(RichText::new("Rien en cours. Clique sur un son ou utilise un raccourci : plusieurs sons peuvent jouer en même temps.").color(TEXT_3));
            return;
        }
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for v in voices.iter().rev() {
                    self.voice_card(ui, v);
                    ui.add_space(GAP - 6.0);
                }
            });
    }

    fn voice_card(&mut self, ui: &mut Ui, v: &VoiceInfo) {
        let engine = self.core.engine.clone();
        Frame::new()
            .fill(SURFACE_2)
            .corner_radius(RADIUS)
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if icon_button(ui, I_STOP, "Arrêter", false).clicked() {
                            engine.stop(v.id);
                        }
                        if icon_button(
                            ui,
                            I_SAVE,
                            "Garder ce volume et cette vitesse comme réglages du son",
                            false,
                        )
                        .clicked()
                        {
                            self.keep_voice_settings(v);
                        }
                        if icon_button(ui, I_LOOP, "Boucle", v.looping).clicked() {
                            engine.set_loop(v.id, !v.looping);
                        }
                        let (icon, label) = if v.paused {
                            (I_PLAY, "Reprendre")
                        } else {
                            (I_PAUSE, "Pause")
                        };
                        if icon_button(ui, icon, label, false).clicked() {
                            engine.set_paused(v.id, !v.paused);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let color = if v.paused { TEXT_3 } else { TEXT };
                            ui.add(
                                Label::new(RichText::new(&v.title).family(strong()).color(color))
                                    .truncate(),
                            );
                        });
                    });
                });
                // Position: dragged locally, sent on release.
                ui.horizontal(|ui| {
                    let time = format!(
                        "{} / {}",
                        crate::model::format_time(v.position),
                        crate::model::format_time(v.duration)
                    );
                    let time_w = 78.0;
                    let mut at = self
                        .seeking
                        .get(&v.id)
                        .copied()
                        .unwrap_or(if v.duration > 0.0 {
                            (v.position / v.duration) as f32
                        } else {
                            0.0
                        });
                    let r = slider(
                        ui,
                        &mut at,
                        0.0..=1.0,
                        ui.available_width() - time_w - GAP,
                        &format!("Position dans {}", v.title),
                    );
                    if r.dragged() {
                        self.seeking.insert(v.id, at);
                    } else if r.changed() || r.drag_stopped() {
                        self.seeking.remove(&v.id);
                        engine.seek(v.id, f64::from(at) * v.duration);
                    }
                    ui.label(RichText::new(time).color(TEXT_3).size(12.0));
                });
                let label_w = 52.0;
                let value_w = 44.0;
                ui.horizontal(|ui| {
                    fixed_label(
                        ui,
                        label_w,
                        RichText::new("Volume").color(TEXT_3).size(12.0),
                    );
                    let mut vol = v.volume;
                    if slider(
                        ui,
                        &mut vol,
                        0.0..=2.0,
                        ui.available_width() - value_w - GAP,
                        "Volume",
                    )
                    .changed()
                    {
                        engine.set_volume(v.id, vol);
                    }
                    ui.label(RichText::new(pct(vol)).color(TEXT_2).size(12.0));
                });
                ui.horizontal(|ui| {
                    fixed_label(
                        ui,
                        label_w,
                        RichText::new("Vitesse").color(TEXT_3).size(12.0),
                    );
                    let mut rate = v.rate;
                    if slider(
                        ui,
                        &mut rate,
                        0.5..=2.0,
                        ui.available_width() - value_w - GAP,
                        "Vitesse",
                    )
                    .changed()
                    {
                        engine.set_rate(v.id, (rate * 20.0).round() / 20.0);
                    }
                    ui.label(RichText::new(rate_label(v.rate)).color(TEXT_2).size(12.0));
                });
            });
    }

    fn keep_voice_settings(&mut self, v: &VoiceInfo) {
        let Some(title) = self.core.edit(|lib| {
            let s = lib.sound_mut(&v.sound_id)?;
            s.volume = f64::from((v.volume * 100.0).round() / 100.0);
            s.rate = f64::from((v.rate * 100.0).round() / 100.0);
            Some(s.title.clone())
        }) else {
            return;
        };
        self.core.save();
        self.core.toast(
            format!(
                "Réglages gardés pour « {title} » : {}, {}",
                pct(v.volume),
                rate_label(v.rate)
            ),
            false,
        );
    }

    fn mixer(&mut self, ui: &mut Ui, lib: &Library) {
        let s = &lib.settings;
        let filter = s.cable_device == Cable::Filter;
        let label_w = 128.0;
        let value_w = 44.0;
        let row =
            |ui: &mut Ui, label: &str, value: f64, enabled: bool, hint: &str| -> Option<f32> {
                let mut changed = None;
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        fixed_label(ui, label_w, RichText::new(label).color(TEXT_2))
                            .on_hover_text(hint);
                        let mut v = value as f32;
                        if slider(
                            ui,
                            &mut v,
                            0.0..=2.0,
                            ui.available_width() - value_w - GAP,
                            label,
                        )
                        .changed()
                        {
                            changed = Some((v * 100.0).round() / 100.0);
                        }
                        ui.label(RichText::new(pct(v)).color(TEXT_2).size(12.0));
                    });
                });
                changed
            };
        let monitor = row(ui, "Casque", s.monitor_volume, true, "Ce que tu entends");
        let cable = row(
            ui,
            "Sons dans le micro",
            s.cable_volume,
            true,
            "Le volume des sons pour les autres",
        );
        let voice = row(
            ui,
            "Ta voix",
            s.mic_volume,
            !filter,
            if filter {
                "Avec le filtre, ta voix passe déjà par ton vrai micro : l’appli n’y touche pas."
            } else {
                "Ta voix, mélangée dans le micro virtuel"
            },
        );
        if monitor.is_some() || cable.is_some() || voice.is_some() {
            self.core.edit(|lib| {
                let s = &mut lib.settings;
                s.monitor_volume = monitor.map_or(s.monitor_volume, f64::from);
                s.cable_volume = cable.map_or(s.cable_volume, f64::from);
                s.mic_volume = voice.map_or(s.mic_volume, f64::from);
            });
            if let Some(v) = monitor {
                self.core.engine.set_bus_volume(MONITOR, v);
            }
            if let Some(v) = cable {
                self.core.engine.set_bus_volume(CABLE, v);
            }
            if let Some(v) = voice {
                self.core.engine.set_mic_volume(v);
            }
            self.core.save();
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new("Ce que les autres entendent")
                .color(TEXT_3)
                .size(12.0),
        );
        let level = self.core.engine.take_level(CABLE);
        self.meter = level.max(self.meter * 0.82);
        if self.meter < 1e-4 {
            self.meter = 0.0;
        }
        meter(ui, self.meter, "Niveau des sons envoyés aux autres");
    }

    // ---------- Overlays ----------

    fn drop_overlay(&self, ctx: &Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            Id::unique("drop"),
        ));
        painter.rect_filled(screen, 0, BG.gamma_multiply(0.85));
        painter.rect_stroke(
            screen.shrink(16.0),
            10,
            Stroke::new(2.0, ACCENT),
            StrokeKind::Inside,
        );
        painter.text(
            screen.center(),
            Align2::CENTER_CENTER,
            "Dépose tes fichiers audio ou dossiers pour les ajouter",
            egui::FontId::new(17.0, strong()),
            TEXT,
        );
    }

    fn toasts_ui(&mut self, ctx: &Context) {
        let now = std::time::Instant::now();
        self.toasts.retain(|t| {
            now.duration_since(t.at) < Duration::from_millis(if t.error { 8000 } else { 3500 })
        });
        while self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        if self.toasts.is_empty() {
            return;
        }
        let next = self
            .toasts
            .iter()
            .map(|t| {
                Duration::from_millis(if t.error { 8000 } else { 3500 })
                    .saturating_sub(now.duration_since(t.at))
            })
            .min();
        if let Some(d) = next {
            ctx.request_repaint_after(d);
        }
        let mut dismiss = None;
        egui::Area::new(Id::unique("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -16.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for (i, t) in self.toasts.iter().enumerate() {
                    let r = Frame::new()
                        .fill(SURFACE_3)
                        .stroke(Stroke::new(1.0, if t.error { DANGER } else { LINE_STRONG }))
                        .corner_radius(RADIUS)
                        .inner_margin(Margin::symmetric(12, 9))
                        .shadow(egui::Shadow {
                            offset: [0, 4],
                            blur: 14,
                            spread: 0,
                            color: egui::Color32::from_black_alpha(110),
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(356.0);
                            ui.add(
                                Label::new(RichText::new(&t.text).color(if t.error {
                                    DANGER
                                } else {
                                    TEXT
                                }))
                                .wrap(),
                            );
                        })
                        .response
                        .interact(Sense::click());
                    if r.clicked() {
                        dismiss = Some(i);
                    }
                }
            });
        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }

    /// Ctrl+V on the window (no text field focused) with a link: the link dialog, filled in.
    fn handle_paste(&mut self, ctx: &Context) {
        if !self.idle(ctx) {
            return;
        }
        let pasted = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Paste(t) => Some(t.trim().to_string()),
                _ => None,
            })
        });
        if let Some(url) = pasted.filter(|t| t.len() > 8 && t[..8].eq_ignore_ascii_case("https://"))
        {
            self.link = Some(LinkDialog::new(&url));
        }
    }

    fn handle_drop(&mut self, ctx: &Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let core = self.core.clone();
        let category = self.view_category();
        std::thread::spawn(move || {
            let paths = store::expand_paths(&dropped);
            core.add_paths(&paths, category.as_deref());
        });
    }
}

impl eframe::App for App {
    /// Runs even while the window is hidden in the notification area.
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.hide_at_start) {
            self.hide(ctx);
        }
        if self.quitting.load(Ordering::Relaxed) {
            // Sent again at each frame (each one asks for the next) until the window closes.
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        let Some(tray) = &self.tray else { return };
        if ctx.input(|i| i.viewport().close_requested())
            && self.core.read(|l| l.settings.close_to_tray)
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide(ctx);
            if !self.core.read(|l| l.settings.tray_tip_shown) {
                self.core.edit(|l| l.settings.tray_tip_shown = true);
                self.core.save();
                tray.notify(
                    "Bruitage reste ouvert",
                    "Tes raccourcis marchent toujours. Clic sur l’icône pour rouvrir la fenêtre, clic droit pour quitter.",
                );
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if self.hidden.load(Ordering::Relaxed) {
            return; // shown again, the next frame builds everything
        }
        let ctx = ui.ctx().clone();
        if let Some(result) = self.capture.as_mut().and_then(|c| c.poll(&ctx)) {
            self.finish_capture(result);
        }
        self.toasts.extend(self.core.take_toasts());
        if self.capture.is_none()
            && ctx
                .input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::F)))
        {
            self.focus_search = true;
        }
        if self.idle(&ctx) {
            if !self.selected.is_empty() && ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.selected.clear();
            }
            if ctx
                .input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::A)))
            {
                let sounds = &self.snap.lib.sounds;
                self.selected = self.visible.iter().map(|&i| sounds[i].id.clone()).collect();
            }
        }
        self.handle_drop(&ctx);
        self.handle_paste(&ctx);

        let rev = self.core.revision();
        if rev != self.rev {
            self.rev = rev;
            self.snap = Arc::new(self.core.snapshot());
            let ids: HashSet<&String> = self.snap.lib.sounds.iter().map(|s| &s.id).collect();
            self.selected.retain(|id| ids.contains(id));
        }
        let snap = self.snap.clone();
        let lib = &snap.lib;
        let status = self.core.engine.status();
        let voices = self.core.engine.voices();
        self.seeking
            .retain(|id, _| voices.iter().any(|v| v.id == *id));

        self.top_bar(ui);
        self.banner(ui, lib, &status);
        Panel::left("categories")
            .resizable(false)
            .exact_size(212.0)
            .show_separator_line(false)
            .frame(Frame::new().fill(BG).inner_margin(Margin {
                left: 12,
                right: 8,
                top: 4,
                bottom: 0,
            }))
            .show(ui, |ui| self.sidebar(ui, &snap));
        Panel::right("playing")
            .resizable(false)
            .exact_size(336.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(Margin::symmetric(14, 0))
                    .stroke(Stroke::new(1.0, LINE)),
            )
            .show(ui, |ui| self.playing(ui, lib, &voices));
        CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin {
                left: 8,
                right: 16,
                top: 4,
                bottom: 0,
            }))
            .show(ui, |ui| self.grid(ui, &snap, &voices));

        self.dialogs(&ctx, &snap, &status);
        self.drop_overlay(&ctx);
        self.toasts_ui(&ctx);
        if !voices.is_empty() || self.meter > 0.0 || self.capture.is_some() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(tray) = &self.tray {
            tray.remove();
        }
        self.core.shutdown();
    }
}
