// The sound editor, the settings, and confirmations.
use eframe::egui::{
    self, Button, ComboBox, Context, Frame, Grid, Id, Key, Margin, Modal, RichText, ScrollArea,
    Stroke, TextEdit, Ui, vec2,
};

use super::capture::{Capture, Target};
use super::theme::*;
use super::widgets::*;
use super::{App, View};
use crate::audio::engine::Status;
use crate::filter::FilterAction;
use crate::keys::format_accelerator;
use crate::model::{
    Action, Cable, Device, GlobalAction, Library, Mode, Sound, filter_message, is_virtual_cable,
    resolve_device,
};
use crate::state::lock;

pub struct Editor {
    pub id: String,
    title: String,
    volume: f32,
    rate: f32,
    mode: Mode,
    favorite: bool,
    hotkey: Option<String>,
    error: Option<String>,
    categories: Vec<(String, String, bool)>, // id, name, member
    path: String,
    missing: bool,
}

impl Editor {
    pub fn open(sound: &Sound, lib: &Library, missing: bool) -> Editor {
        Editor {
            id: sound.id.clone(),
            title: sound.title.clone(),
            volume: sound.volume as f32,
            rate: sound.rate as f32,
            mode: sound.mode,
            favorite: sound.favorite,
            hotkey: sound.hotkey.clone(),
            error: None,
            categories: lib
                .categories
                .iter()
                .map(|c| {
                    (
                        c.id.clone(),
                        c.name.clone(),
                        c.sound_ids.contains(&sound.id),
                    )
                })
                .collect(),
            path: sound.path.clone(),
            missing,
        }
    }
}

pub enum Confirm {
    RemoveCategory { id: String, name: String },
    RemoveSound { id: String, title: String },
}

fn dialog_frame() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE_STRONG))
        .corner_radius(10)
        .inner_margin(Margin::same(22))
        .shadow(egui::Shadow {
            offset: [0, 10],
            blur: 32,
            spread: 0,
            color: egui::Color32::from_black_alpha(150),
        })
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(title).family(strong()).size(15.0).color(TEXT));
    ui.add_space(2.0);
}

const MODES: [Mode; 3] = [Mode::Overlap, Mode::Restart, Mode::Toggle];

fn hotkey_button(ui: &mut Ui, hotkey: Option<&String>, capturing: bool, label: &str) -> bool {
    let text = if capturing {
        RichText::new("Appuie sur une touche (Échap : annuler)").color(ACCENT)
    } else if let Some(h) = hotkey {
        RichText::new(format_accelerator(h)).family(strong())
    } else {
        RichText::new("Définir un raccourci").color(TEXT_3)
    };
    let stroke = Stroke::new(1.0, if capturing { ACCENT } else { LINE });
    ui.add(Button::new(text).min_size(vec2(250.0, 30.0)).stroke(stroke))
        .on_hover_text(label)
        .clicked()
}

/// Saved device -> choice in a list of names; "" is the Windows default.
fn device_combo(ui: &mut Ui, id: &str, saved: &mut Device, names: &[String], default_label: &str) {
    let current = resolve_device(names, saved).unwrap_or_default();
    let text = if current.is_empty() {
        default_label.to_string()
    } else {
        current.clone()
    };
    ComboBox::from_id_salt(id)
        .selected_text(text)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(current.is_empty(), default_label)
                .clicked()
            {
                *saved = Device::default();
            }
            for n in names {
                if ui.selectable_label(current == *n, n).clicked() {
                    *saved = Device::named(n);
                }
            }
        });
}

fn cable_combo(ui: &mut Ui, cable: &mut Cable, names: &[String]) {
    const FILTER: &str = "Mon micro, avec le filtre (comme Soundpad)";
    let current = match cable {
        Cable::Device(d) => resolve_device(names, d).filter(|n| !n.is_empty()),
        _ => None,
    };
    let text = match (&cable, &current) {
        (Cable::Filter, _) => FILTER.to_string(),
        (_, Some(n)) => n.clone(),
        _ => "Désactivé".to_string(),
    };
    ComboBox::from_id_salt("cable")
        .selected_text(text)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(*cable != Cable::Filter && current.is_none(), "Désactivé")
                .clicked()
            {
                *cable = Cable::Off;
            }
            if ui
                .selectable_label(*cable == Cable::Filter, FILTER)
                .clicked()
            {
                *cable = Cable::Filter;
            }
            for n in names {
                if ui
                    .selectable_label(current.as_ref() == Some(n), n)
                    .clicked()
                {
                    *cable = Cable::Device(Device::named(n));
                }
            }
        });
}

impl App {
    pub(super) fn dialogs(&mut self, ctx: &Context, lib: &Library, status: &Status) {
        if self.editor.is_some() {
            self.editor_ui(ctx, lib);
        }
        if self.settings {
            self.settings_ui(ctx, lib, status);
        }
        if self.confirm.is_some() {
            self.confirm_ui(ctx);
        }
    }

    fn escape(&self, ctx: &Context) -> bool {
        self.capture.is_none() && ctx.input(|i| i.key_pressed(Key::Escape))
    }

    pub(super) fn start_capture(&mut self, target: Target) {
        self.core.suspend_hotkeys();
        self.capture = Some(Capture::start(target));
    }

    /// `result`: the hotkey typed, or None when cancelled. Then every hotkey is registered again.
    pub(super) fn finish_capture(&mut self, result: Option<String>) {
        let Some(capture) = self.capture.take() else {
            return;
        };
        if let Some(accel) = result {
            match capture.target {
                Target::Sound => {
                    if let Some(ed) = &mut self.editor {
                        let id = ed.id.clone();
                        match self
                            .core
                            .read(|l| l.hotkey_owner(&accel, &Action::Play(id)))
                        {
                            Some(owner) => {
                                ed.error = Some(format!(
                                    "{} est déjà utilisé par {owner}.",
                                    format_accelerator(&accel)
                                ))
                            }
                            None => {
                                ed.hotkey = Some(accel);
                                ed.error = None;
                            }
                        }
                    }
                }
                Target::Global(a) => {
                    let owner = self.core.edit(|lib| {
                        let owner = lib.hotkey_owner(&accel, &Action::Global(a));
                        if owner.is_none() {
                            *lib.settings.hotkeys.get_mut(a) = Some(accel.clone());
                        }
                        owner
                    });
                    match owner {
                        Some(owner) => {
                            self.settings_error = Some(format!(
                                "{} est déjà utilisé par {owner}.",
                                format_accelerator(&accel)
                            ))
                        }
                        None => {
                            self.settings_error = None;
                            self.core.save();
                        }
                    }
                }
            }
        }
        self.core.apply_hotkeys();
    }

    // ---------- Sound editor ----------

    fn editor_ui(&mut self, ctx: &Context, lib: &Library) {
        let capturing = self
            .capture
            .as_ref()
            .is_some_and(|c| c.target == Target::Sound);
        let (mut save, mut cancel, mut preview, mut remove, mut capture) =
            (false, false, false, false, false);
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        Modal::new(Id::unique("editor"))
            .frame(dialog_frame())
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(RichText::new("Modifier le son").heading());
                ui.add_space(12.0);
                field_label(ui, "Titre");
                ui.add(
                    TextEdit::singleline(&mut ed.title)
                        .char_limit(200)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(10.0);
                ui.columns(2, |cols| {
                    field_label(
                        &mut cols[0],
                        &format!("Volume par défaut   {}", pct(ed.volume)),
                    );
                    let w = cols[0].available_width() - 8.0;
                    if slider(
                        &mut cols[0],
                        &mut ed.volume,
                        0.0..=2.0,
                        w,
                        "Volume par défaut",
                    )
                    .changed()
                    {
                        ed.volume = (ed.volume * 100.0).round() / 100.0;
                    }
                    field_label(
                        &mut cols[1],
                        &format!("Vitesse par défaut   {}", rate_label(ed.rate)),
                    );
                    let w = cols[1].available_width();
                    if slider(
                        &mut cols[1],
                        &mut ed.rate,
                        0.5..=2.0,
                        w,
                        "Vitesse par défaut",
                    )
                    .changed()
                    {
                        ed.rate = (ed.rate * 20.0).round() / 20.0;
                    }
                });
                ui.add_space(10.0);
                field_label(ui, "Si on le relance pendant qu’il joue");
                ComboBox::from_id_salt("mode")
                    .selected_text(ed.mode.label())
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut ed.mode, Mode::Default, Mode::Default.label());
                        for m in MODES {
                            ui.selectable_value(&mut ed.mode, m, m.label());
                        }
                    });
                ui.add_space(10.0);
                field_label(ui, "Raccourci clavier global");
                ui.horizontal(|ui| {
                    capture = hotkey_button(
                        ui,
                        ed.hotkey.as_ref(),
                        capturing,
                        "Raccourci clavier global",
                    );
                    if ed.hotkey.is_some() && ui.button("Effacer").clicked() {
                        ed.hotkey = None;
                        ed.error = None;
                    }
                });
                if let Some(e) = &ed.error {
                    ui.label(RichText::new(e).color(DANGER));
                }
                ui.add_space(10.0);
                field_label(ui, "Catégories");
                if ed.categories.is_empty() {
                    hint(
                        ui,
                        "Aucune catégorie : crée-en une dans la colonne de gauche.",
                    );
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for (_, name, member) in &mut ed.categories {
                            ui.checkbox(member, name.as_str());
                        }
                    });
                }
                ui.add_space(6.0);
                ui.checkbox(&mut ed.favorite, "Favori");
                ui.add_space(6.0);
                let path = if ed.missing {
                    format!("{} (introuvable)", ed.path)
                } else {
                    ed.path.clone()
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(path)
                            .color(if ed.missing { DANGER } else { TEXT_3 })
                            .size(12.0),
                    )
                    .wrap(),
                );
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    remove = danger(ui, "Retirer de la bibliothèque").clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        save = primary(ui, "Enregistrer").clicked();
                        cancel = ui.button("Annuler").clicked();
                        preview = icon_text(ui, I_PLAY, "Écouter").clicked();
                    });
                });
            });
        if capture {
            self.start_capture(Target::Sound);
        }
        if remove {
            let ed = self.editor.as_ref().unwrap();
            self.confirm = Some(Confirm::RemoveSound {
                id: ed.id.clone(),
                title: ed.title.clone(),
            });
        }
        if preview
            && let Some(ed) = &self.editor
            && let Some(s) = lib.sound(&ed.id)
        {
            let mut s = s.clone();
            s.volume = f64::from(ed.volume);
            s.rate = f64::from(ed.rate);
            self.core.preview(&s);
        }
        if save {
            self.apply_editor();
        }
        if cancel || (self.confirm.is_none() && self.escape(ctx)) {
            if capturing {
                self.finish_capture(None);
            }
            self.editor = None;
        }
    }

    fn apply_editor(&mut self) {
        if self.capture.is_some() {
            self.finish_capture(None);
        }
        let Some(ed) = self.editor.take() else { return };
        let Some(hotkey_changed) = self.core.edit(|lib| {
            let s = lib.sound_mut(&ed.id)?;
            let title = ed.title.trim();
            if !title.is_empty() {
                s.title = title.to_string();
            }
            s.volume = f64::from(ed.volume);
            s.rate = f64::from(ed.rate);
            s.mode = ed.mode;
            s.favorite = ed.favorite;
            let changed = s.hotkey != ed.hotkey;
            s.hotkey = ed.hotkey.clone();
            for (id, _, member) in &ed.categories {
                if let Some(c) = lib.categories.iter_mut().find(|c| c.id == *id) {
                    let has = c.sound_ids.contains(&ed.id);
                    if *member && !has {
                        c.sound_ids.push(ed.id.clone());
                    } else if !*member && has {
                        c.sound_ids.retain(|x| *x != ed.id);
                    }
                }
            }
            Some(changed)
        }) else {
            return;
        };
        self.core.save_now();
        if hotkey_changed {
            self.core.apply_hotkeys();
        }
    }

    // ---------- Settings ----------

    fn settings_ui(&mut self, ctx: &Context, lib: &Library, status: &Status) {
        let before = lib.settings.clone();
        let mut s = before.clone();
        let capturing = self.capture.as_ref().and_then(|c| match c.target {
            Target::Global(a) => Some(a),
            Target::Sound => None,
        });
        let (filter_status, busy) = {
            let f = lock(&self.core.filter);
            (f.status.clone(), f.busy)
        };
        let live = self.core.engine.filter_live();
        let (mut close, mut import, mut capture, mut filter_action) = (false, false, None, None);
        let max_height = ctx.content_rect().height() - 150.0;
        Modal::new(Id::unique("settings")).frame(dialog_frame()).backdrop_color(egui::Color32::from_black_alpha(150)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new("Réglages").heading());
            ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(548.0);
                let filter = s.cable_device == Cable::Filter;

                section(ui, "Sorties audio");
                field_label(ui, "Casque : ce que tu entends");
                device_combo(ui, "monitor", &mut s.monitor_device, &status.outputs, "Sortie par défaut de Windows");
                ui.add_space(8.0);
                field_label(ui, "Envoi aux autres : ce que Discord et les jeux entendent");
                cable_combo(ui, &mut s.cable_device, &status.outputs);
                let cable_found = status.outputs.iter().any(|n| is_virtual_cable(n));
                hint(
                    ui,
                    if filter {
                        "Windows ajoute les sons à ton vrai micro : dans Discord, garde ton micro habituel. Coupe la suppression de bruit de Discord (Krisp), sinon elle efface la musique."
                    } else if cable_found {
                        "Dans Discord : Paramètres, Voix et vidéo, Périphérique d’entrée : « CABLE Output (VB-Audio Virtual Cable) ». Coupe aussi la suppression de bruit de Discord, sinon elle efface la musique."
                    } else {
                        "Aucun câble virtuel trouvé. Choisis plutôt « Mon micro, avec le filtre », ou installe VB-Cable (gratuit, vb-audio.com), redémarre le PC, puis choisis ici « CABLE Input »."
                    },
                );
                if filter {
                    ui.add_space(6.0);
                    Frame::new().fill(SURFACE_2).corner_radius(RADIUS).inner_margin(Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let text = if busy { "Windows demande l’autorisation dans sa fenêtre de contrôle de compte…".to_string() } else { filter_message(filter_status.as_ref(), live) };
                        ui.add(egui::Label::new(RichText::new(text).color(TEXT_2)).wrap());
                        if let Some(st) = filter_status.as_ref().filter(|st| st.ok) {
                            let can_install = st.eligible.iter().any(|n| !st.installed.contains(n));
                            let can_remove = !st.installed.is_empty() || st.dll;
                            ui.add_space(4.0);
                            ui.add_enabled_ui(!busy, |ui| {
                                ui.horizontal(|ui| {
                                    if can_install && primary(ui, "Installer le filtre").clicked() {
                                        filter_action = Some(FilterAction::Install);
                                    }
                                    if can_remove && ui.button("Retirer le filtre").clicked() {
                                        filter_action = Some(FilterAction::Uninstall);
                                    }
                                });
                            });
                        }
                    });
                }

                section(ui, "Ta voix");
                if filter {
                    hint(ui, "Avec le filtre, ta voix passe déjà par ton vrai micro : l’appli n’y touche pas.");
                }
                ui.add_enabled_ui(!filter, |ui| {
                    ui.checkbox(&mut s.mic_enabled, "Mélanger ma voix dans le micro virtuel");
                    field_label(ui, "Micro");
                    device_combo(ui, "mic", &mut s.mic_device, &status.inputs, "Micro par défaut de Windows");
                    ui.checkbox(&mut s.mic_noise_suppression, "Réduction de bruit sur ma voix");
                });

                section(ui, "Lecture");
                field_label(ui, "Quand on relance un son qui joue déjà");
                ComboBox::from_id_salt("default-mode").selected_text(s.default_mode.label()).width(ui.available_width()).show_ui(ui, |ui| {
                    for m in MODES {
                        ui.selectable_value(&mut s.default_mode, m, m.label());
                    }
                });
                ui.checkbox(&mut s.exclusive, "Un seul son à la fois : un nouveau son coupe les autres (comme Soundpad)");
                ui.checkbox(&mut s.normalize, "Égaliser le volume des sons (les sons trop forts sont baissés, les faibles remontés)");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Sons simultanés au maximum").color(TEXT_2));
                    ui.add(egui::DragValue::new(&mut s.max_voices).range(1..=64));
                });

                section(ui, "Raccourcis globaux");
                Grid::new("global-hotkeys").num_columns(3).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    for a in GlobalAction::ALL {
                        ui.label(RichText::new(a.label()).color(TEXT_2));
                        if hotkey_button(ui, s.hotkeys.get(a).as_ref(), capturing == Some(a), a.label()) {
                            capture = Some(a);
                        }
                        if s.hotkeys.get(a).is_some() && ui.button("Effacer").clicked() {
                            *s.hotkeys.get_mut(a) = None;
                        }
                        ui.end_row();
                    }
                });
                if let Some(e) = &self.settings_error {
                    ui.label(RichText::new(e).color(DANGER));
                }
                hint(
                    ui,
                    "Un raccourci global marche partout, même en jeu, mais sa touche n’arrive plus aux autres applis. Préfère le pavé numérique, Pause, F13 à F24 ou une combinaison avec Ctrl ou Alt.",
                );

                section(ui, "Bibliothèque");
                hint(ui, "Les sons, catégories et raccourcis de Soundpad. Son fichier est seulement lu, jamais modifié.");
                import = ui.button("Importer depuis Soundpad").clicked();
                hint(ui, &format!("Ta bibliothèque : {}", crate::store::data_dir().join("library.json").display()));
                ui.add_space(4.0);
            });
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = primary(ui, "Fermer").clicked();
            });
        });

        if s != before {
            let audio = s.monitor_device != before.monitor_device
                || s.cable_device != before.cable_device
                || s.mic_device != before.mic_device
                || s.mic_enabled != before.mic_enabled
                || s.mic_noise_suppression != before.mic_noise_suppression;
            let hotkeys = s.hotkeys != before.hotkeys;
            let to_filter = s.cable_device == Cable::Filter && before.cable_device != Cable::Filter;
            self.core.engine.set_normalize(s.normalize);
            self.core.engine.set_max_voices(s.max_voices as usize);
            self.core.edit(|lib| lib.settings = s);
            self.core.save();
            if audio {
                self.core.configure_audio();
            }
            if hotkeys {
                self.core.apply_hotkeys();
            }
            if to_filter {
                let core = self.core.clone();
                std::thread::spawn(move || core.refresh_filter());
            }
        }
        if let Some(a) = capture {
            self.settings_error = None;
            self.start_capture(Target::Global(a));
        }
        if let Some(action) = filter_action {
            self.core.run_filter(action);
        }
        if import {
            self.import_soundpad();
        }
        if close || self.escape(ctx) {
            if capturing.is_some() {
                self.finish_capture(None);
            }
            self.settings = false;
        }
    }

    // ---------- Confirmation ----------

    fn confirm_ui(&mut self, ctx: &Context) {
        let Some(c) = &self.confirm else { return };
        let (title, text, verb) = match c {
            Confirm::RemoveCategory { name, .. } => (
                format!("Supprimer la catégorie « {name} » ?"),
                "Ses sons restent dans la bibliothèque.",
                "Supprimer",
            ),
            Confirm::RemoveSound { title, .. } => (
                format!("Retirer « {title} » de la bibliothèque ?"),
                "Le fichier audio reste sur le disque.",
                "Retirer",
            ),
        };
        let (mut yes, mut no) = (false, false);
        Modal::new(Id::unique("confirm"))
            .frame(dialog_frame())
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.add(egui::Label::new(RichText::new(title).heading()).wrap());
                ui.add_space(4.0);
                ui.label(RichText::new(text).color(TEXT_2));
                ui.add_space(16.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    yes = ui
                        .add(
                            Button::new(RichText::new(verb).color(ACCENT_TEXT).family(strong()))
                                .fill(DANGER)
                                .stroke(Stroke::NONE),
                        )
                        .clicked();
                    no = ui.button("Annuler").clicked();
                });
            });
        if no || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.confirm = None;
            return;
        }
        if !yes {
            return;
        }
        match self.confirm.take() {
            Some(Confirm::RemoveCategory { id, .. }) => {
                self.core.edit(|lib| lib.categories.retain(|c| c.id != id));
                if self.view == View::Category(id) {
                    self.view = View::All;
                }
                self.core.save_now();
            }
            Some(Confirm::RemoveSound { id, .. }) => {
                self.editor = None;
                self.core.remove_sound(&id);
            }
            None => {}
        }
    }
}
