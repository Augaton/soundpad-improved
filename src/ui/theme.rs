// Look: cool dark greys, one amber accent, soft 6 px corners, Windows' own fonts.
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow,
    Stroke, TextStyle, Vec2, Visuals, epaint::text::FontTweak,
};

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const BG: Color32 = hex(0x15171a);
pub const SURFACE: Color32 = hex(0x1b1e22);
pub const SURFACE_2: Color32 = hex(0x22262b);
pub const SURFACE_3: Color32 = hex(0x2b3036);
pub const HOVER: Color32 = hex(0x343a41);
pub const LINE: Color32 = hex(0x2b3036);
pub const LINE_STRONG: Color32 = hex(0x464d55);
pub const TEXT: Color32 = hex(0xe6e8eb);
pub const TEXT_2: Color32 = hex(0xb0b6bd);
pub const TEXT_3: Color32 = hex(0x8d959e);
pub const ACCENT: Color32 = hex(0xe0a046);
pub const ACCENT_TEXT: Color32 = hex(0x1c1407);
pub const ACCENT_SOFT: Color32 = hex(0x3a3123); // the accent at 14 % over SURFACE_2
pub const DANGER: Color32 = hex(0xe2766a);

pub const RADIUS: u8 = 6;
pub const GAP: f32 = 8.0;

// Segoe Fluent Icons (Windows 11), same code points in Segoe MDL2 Assets (Windows 10).
pub const I_SEARCH: &str = "\u{E721}";
pub const I_ADD: &str = "\u{E710}";
pub const I_SETTINGS: &str = "\u{E713}";
pub const I_STOP: &str = "\u{E71A}";
pub const I_PLAY: &str = "\u{E768}";
pub const I_PAUSE: &str = "\u{E769}";
pub const I_LOOP: &str = "\u{E8EE}";
pub const I_CLOSE: &str = "\u{E711}";
pub const I_STAR: &str = "\u{E735}";
pub const I_MORE: &str = "\u{E712}";
pub const I_SAVE: &str = "\u{E74E}";
pub const I_WARNING: &str = "\u{E7BA}";

pub fn strong() -> FontFamily {
    FontFamily::Name("strong".into())
}

pub fn icons() -> FontFamily {
    FontFamily::Name("icons".into())
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, icons())
}

fn system_font(names: &[&str]) -> Option<Vec<u8>> {
    let dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    names.iter().find_map(|n| std::fs::read(dir.join(n)).ok())
}

/// A dark title bar, whatever the Windows theme (light app mode included).
pub fn dark_title_bar(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    use windows::core::BOOL;
    if let Ok(handle) = cc.window_handle()
        && let RawWindowHandle::Win32(h) = handle.as_raw()
    {
        let on = BOOL(1);
        let _ = unsafe {
            DwmSetWindowAttribute(
                HWND(h.hwnd.get() as _),
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const on).cast(),
                size_of::<BOOL>() as u32,
            )
        };
    }
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, files: &[&str], tweak: FontTweak| -> bool {
        let Some(bytes) = system_font(files) else {
            return false;
        };
        fonts.font_data.insert(
            name.into(),
            Arc::new(FontData::from_owned(bytes).tweak(tweak)),
        );
        true
    };
    let body = add("segoe", &["segoeui.ttf"], FontTweak::default());
    let semibold = add(
        "segoe-sb",
        &["seguisb.ttf", "segoeuib.ttf"],
        FontTweak::default(),
    );
    let icon = add(
        "icons",
        &["SegoeIcons.ttf", "segmdl2.ttf"],
        FontTweak {
            y_offset_factor: 0.08,
            ..Default::default()
        },
    );
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    if body {
        proportional.insert(0, "segoe".into());
    }
    if icon {
        proportional.push("icons".into()); // icons inline in labels and buttons
    }
    let mut strong_list = fonts.families[&FontFamily::Proportional].clone();
    if semibold {
        strong_list.insert(0, "segoe-sb".into());
    }
    fonts.families.insert(strong(), strong_list);
    let mut icon_list = vec![];
    if icon {
        icon_list.push("icons".to_string());
    }
    icon_list.extend(fonts.families[&FontFamily::Proportional].clone());
    fonts.families.insert(icons(), icon_list);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::new(17.0, strong())),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(GAP, 6.0);
        s.button_padding = Vec2::new(12.0, 5.0);
        s.interact_size = Vec2::new(28.0, 30.0);
        s.slider_width = 140.0;
        s.slider_rail_height = 4.0;
        s.combo_height = 320.0;
        s.window_margin = Margin::same(20);
        s.menu_margin = Margin::same(6);
        s.icon_width = 16.0;
        s.icon_width_inner = 10.0;
        style.visuals = visuals();
    });
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    let r = CornerRadius::same(RADIUS);
    v.override_text_color = None;
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, LINE_STRONG);
    v.window_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    v.menu_corner_radius = r;
    v.faint_bg_color = SURFACE;
    v.extreme_bg_color = hex(0x101215); // text fields, slider rails
    v.text_edit_bg_color = Some(hex(0x101215));
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = ACCENT;
    v.error_fg_color = DANGER;
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_2);
    w.noninteractive.corner_radius = r;
    for (state, fill, stroke, text) in [
        (&mut w.inactive, SURFACE_3, LINE, TEXT),
        (&mut w.hovered, HOVER, LINE_STRONG, TEXT),
        (&mut w.active, HOVER, ACCENT, TEXT),
        (&mut w.open, SURFACE_3, LINE_STRONG, TEXT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.5, text);
        state.corner_radius = r;
        state.expansion = 0.0;
    }
    // Slider handles and check marks in the accent.
    w.inactive.fg_stroke = Stroke::new(1.5, TEXT);
    v
}
