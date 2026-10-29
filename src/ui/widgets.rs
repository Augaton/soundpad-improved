// Small building blocks: buttons, sliders, the sound pad, the level meter.
use std::ops::RangeInclusive;

use eframe::egui::{
    self, Align2, Button, Color32, CornerRadius, FontId, Id, Rect, Response, RichText, Role, Sense,
    Stroke, StrokeKind, Ui, Vec2, WidgetInfo, pos2,
    text::{LayoutJob, TextFormat, TextWrapping},
    vec2,
};

use super::theme::*;

pub fn pct(v: f32) -> String {
    format!("{} %", (v * 100.0).round())
}

pub fn rate_label(r: f32) -> String {
    format!("{r:.2}×").replace('.', ",")
}

/// A thin slider: dark rail, amber fill, small light knob. Drag or click to set; arrow keys
/// step by a hundredth of the range. `label` is its accessible name.
pub fn slider(
    ui: &mut Ui,
    value: &mut f32,
    range: RangeInclusive<f32>,
    width: f32,
    label: &str,
) -> Response {
    const KNOB: f32 = 6.0;
    let (lo, hi) = (*range.start(), *range.end());
    let enabled = ui.is_enabled();
    let (rect, mut resp) = ui.allocate_exact_size(
        vec2(width.max(2.0 * KNOB + 10.0), 20.0),
        Sense::click_and_drag(),
    );
    let track = rect.x_range().shrink(KNOB);
    let mut v = value.clamp(lo, hi);
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
    {
        v = lo + ((p.x - track.min) / track.span()).clamp(0.0, 1.0) * (hi - lo);
    }
    if resp.has_focus() {
        let step = (hi - lo) / 100.0;
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
            {
                v = (v + step).min(hi);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
            {
                v = (v - step).max(lo);
            }
        });
    }
    if v != *value {
        *value = v;
        resp.mark_changed();
    }
    resp.widget_info(|| WidgetInfo::slider(enabled, f64::from(v), label));

    let t = if hi > lo { (v - lo) / (hi - lo) } else { 0.0 };
    let x = track.min + t * track.span();
    let y = rect.center().y;
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
    let p = ui.painter();
    p.rect_filled(
        Rect::from_x_y_ranges(rect.x_range(), (y - 2.0)..=(y + 2.0)),
        2,
        SURFACE_3,
    );
    p.rect_filled(
        Rect::from_x_y_ranges(rect.left()..=x, (y - 2.0)..=(y + 2.0)),
        2,
        fade(ACCENT),
    );
    let active = resp.hovered() || resp.dragged();
    p.circle_filled(
        pos2(x, y),
        if active { KNOB + 1.0 } else { KNOB },
        fade(TEXT),
    );
    if resp.has_focus() {
        p.circle_stroke(pos2(x, y), KNOB + 3.0, Stroke::new(1.5, ACCENT));
    }
    resp
}

pub fn primary(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        Button::new(RichText::new(text).color(ACCENT_TEXT).family(strong()))
            .fill(ACCENT)
            .stroke(Stroke::NONE),
    )
}

pub fn danger(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        Button::new(RichText::new(text).color(DANGER))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, LINE_STRONG)),
    )
}

/// A button with an icon and a text.
pub fn icon_text(ui: &mut Ui, icon: &str, text: &str) -> Response {
    let mut job = LayoutJob::default();
    job.append(
        icon,
        0.0,
        TextFormat {
            font_id: icon_font(14.0),
            color: TEXT,
            ..Default::default()
        },
    );
    job.append(
        text,
        8.0,
        TextFormat {
            font_id: FontId::proportional(14.0),
            color: TEXT,
            ..Default::default()
        },
    );
    ui.add(Button::new(job))
}

/// A square icon-only button; `label` is its tooltip and its accessible name.
pub fn icon_button(ui: &mut Ui, icon: &str, label: &str, active: bool) -> Response {
    let enabled = ui.is_enabled();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::Button, enabled, active, label));
    let fill = if resp.is_pointer_button_down_on() {
        HOVER
    } else if resp.hovered() {
        SURFACE_3
    } else if active {
        ACCENT_SOFT
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, RADIUS, fill);
    if resp.has_focus() {
        p.rect_stroke(rect, RADIUS, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
    }
    let color = if !enabled {
        TEXT_3.gamma_multiply(0.5)
    } else if active {
        ACCENT
    } else if resp.hovered() {
        TEXT
    } else {
        TEXT_2
    };
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        icon_font(14.0),
        color,
    );
    resp.on_hover_text(label)
}

/// A key name in a small outlined box ("Num 1").
pub fn key_chip(ui: &Ui, right: f32, center_y: f32, text: &str, failed: bool) -> Rect {
    let font = FontId::proportional(11.5);
    let color = if failed { DANGER } else { TEXT_2 };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = galley.size() + vec2(10.0, 4.0);
    let rect = Rect::from_min_size(pos2(right - size.x, center_y - size.y / 2.0), size);
    let p = ui.painter();
    p.rect_stroke(
        rect,
        4,
        Stroke::new(1.0, if failed { DANGER } else { LINE_STRONG }),
        StrokeKind::Inside,
    );
    p.galley(rect.min + vec2(5.0, 2.0), galley, color);
    if failed {
        p.hline(
            rect.x_range().shrink(3.0),
            rect.center().y,
            Stroke::new(1.0, DANGER),
        );
    }
    rect
}

pub struct Pad<'a> {
    pub title: &'a str,
    pub duration: Option<f64>,
    pub hotkey: Option<String>, // display form
    pub hotkey_failed: bool,
    pub favorite: bool,
    pub missing: bool,
    pub playing: usize,
    pub progress: Option<f32>,
}

pub enum PadAction {
    None,
    Play,
    Edit,
}

pub const PAD_HEIGHT: f32 = 80.0;

/// One sound of the grid: click plays, right click or ••• edits.
pub fn pad(ui: &mut Ui, rect: Rect, id: Id, p: &Pad) -> (PadAction, Response) {
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, p.title));
    let edit_rect = Rect::from_min_size(
        pos2(rect.right() - 32.0, rect.top() + 6.0),
        vec2(26.0, 24.0),
    );
    let edit = ui
        .interact(edit_rect, id.with("edit"), Sense::click())
        .on_hover_text("Modifier");
    edit.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Modifier {}", p.title)));

    let hovered = resp.hovered() || edit.hovered();
    let painter = ui.painter();
    let body = if resp.is_pointer_button_down_on() {
        rect.shrink(1.0)
    } else {
        rect
    };
    let fill = match (p.playing > 0, hovered) {
        (true, _) => ACCENT_SOFT,
        (false, true) => SURFACE_3,
        (false, false) => SURFACE_2,
    };
    painter.rect_filled(body, RADIUS, fill);
    if resp.has_focus() || edit.has_focus() {
        painter.rect_stroke(body, RADIUS, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
    } else if p.playing > 0 {
        painter.rect_stroke(body, RADIUS, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
    } else if p.missing {
        painter.rect_stroke(
            body,
            RADIUS,
            Stroke::new(1.0, LINE_STRONG),
            StrokeKind::Inside,
        );
    }

    // Title: two lines at most.
    let inner = body.shrink2(vec2(12.0, 10.0));
    let mut job = LayoutJob::single_section(
        p.title.to_string(),
        TextFormat {
            font_id: FontId::proportional(14.0),
            color: if p.missing { TEXT_3 } else { TEXT },
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: inner.width() - 22.0,
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let title = painter.layout_job(job);
    painter.galley(inner.min, title, TEXT);

    // Bottom line: favorite, duration or "missing", instance count; the hotkey on the right.
    let y = inner.bottom() - 7.0;
    let mut x = inner.left();
    if p.favorite {
        let r = painter.text(
            pos2(x, y),
            Align2::LEFT_CENTER,
            I_STAR,
            icon_font(11.0),
            ACCENT,
        );
        x = r.right() + 6.0;
    }
    let (meta, color) = if p.missing {
        ("Fichier introuvable".to_string(), DANGER)
    } else {
        (
            p.duration
                .map(crate::model::format_time)
                .unwrap_or_default(),
            TEXT_3,
        )
    };
    let r = painter.text(
        pos2(x, y),
        Align2::LEFT_CENTER,
        meta,
        FontId::proportional(12.0),
        color,
    );
    x = r.right() + 8.0;
    if p.playing > 1 {
        painter.text(
            pos2(x, y),
            Align2::LEFT_CENTER,
            format!("×{}", p.playing),
            FontId::new(12.0, strong()),
            ACCENT,
        );
    }
    if let Some(key) = &p.hotkey {
        let chip = key_chip(ui, inner.right(), y, key, p.hotkey_failed);
        if p.hotkey_failed {
            ui.interact(chip, id.with("key"), Sense::hover())
                .on_hover_text("Raccourci indisponible : une autre application l’utilise déjà");
        }
    }

    if let Some(progress) = p.progress {
        let track = body.shrink2(vec2(RADIUS as f32, 0.0));
        let width = track.width() * progress.clamp(0.0, 1.0);
        let bar = Rect::from_min_size(pos2(track.left(), body.bottom() - 3.0), vec2(width, 2.0));
        ui.painter().rect_filled(bar, 1, ACCENT);
    }

    if hovered || edit.has_focus() {
        let p = ui.painter();
        if edit.hovered() {
            p.rect_filled(edit_rect, RADIUS, HOVER);
        }
        p.text(
            edit_rect.center(),
            Align2::CENTER_CENTER,
            I_MORE,
            icon_font(14.0),
            if edit.hovered() { TEXT } else { TEXT_2 },
        );
    }

    let action = if edit.clicked() || resp.secondary_clicked() {
        PadAction::Edit
    } else if resp.clicked() {
        PadAction::Play
    } else {
        PadAction::None
    };
    (action, resp)
}

/// Peak meter, -60..0 dBFS; red near full scale.
pub fn meter(ui: &mut Ui, level: f32, label: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let db = 20.0 * level.max(1e-6).log10();
    let fill = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    resp.widget_info(|| WidgetInfo::slider(false, f64::from(fill * 100.0), label));
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(3), SURFACE_3);
    if fill > 0.0 {
        let bar = Rect::from_min_size(rect.min, vec2(rect.width() * fill, rect.height()));
        p.rect_filled(
            bar,
            CornerRadius::same(3),
            if db > -1.0 { DANGER } else { ACCENT },
        );
    }
    resp
}

/// A label in a fixed width, left-aligned, so the controls after it line up.
pub fn fixed_label(ui: &mut Ui, width: f32, text: RichText) -> Response {
    ui.allocate_ui_with_layout(
        vec2(width, 20.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.add(egui::Label::new(text).truncate())
        },
    )
    .inner
}

/// A field label, above its control.
pub fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(TEXT_2).size(13.0));
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(TEXT_3).size(12.5));
}
