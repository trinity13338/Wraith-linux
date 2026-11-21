//! Material 3 palette (the same teal scheme as the Android app), the desktop's fonts and icon glyphs.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub primary: Color32, pub on_primary: Color32, pub primary_container: Color32, pub on_primary_container: Color32,
    pub secondary_container: Color32, pub on_secondary_container: Color32,
    pub tertiary: Color32, pub tertiary_container: Color32,
    pub error: Color32, pub error_container: Color32, pub on_error_container: Color32,
    pub surface: Color32, pub text: Color32, pub muted: Color32, pub outline: Color32, pub outline_variant: Color32,
    pub lowest: Color32, pub low: Color32, pub container: Color32, pub high: Color32, pub highest: Color32,
    pub good: Color32, pub warn: Color32,
}

const fn c(hex: u32) -> Color32 { Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8) }

pub const LIGHT: Palette = Palette {
    dark: false,
    primary: c(0x006B5B), on_primary: c(0xFFFFFF), primary_container: c(0x9EF2DC), on_primary_container: c(0x00201A),
    secondary_container: c(0xCDE8DF), on_secondary_container: c(0x06201A),
    tertiary: c(0x426278), tertiary_container: c(0xC8E6FF),
    error: c(0xBA1A1A), error_container: c(0xFFDAD6), on_error_container: c(0x410002),
    surface: c(0xF5FBF7), text: c(0x171D1B), muted: c(0x3F4945), outline: c(0x6F7975), outline_variant: c(0xBFC9C4),
    lowest: c(0xFFFFFF), low: c(0xEFF5F1), container: c(0xE9EFEB), high: c(0xE3EAE6), highest: c(0xDEE4E0),
    good: c(0x1E7D4A), warn: c(0x9A6400),
};
pub const DARK: Palette = Palette {
    dark: true,
    primary: c(0x82D5C0), on_primary: c(0x00382E), primary_container: c(0x005144), on_primary_container: c(0x9EF2DC),
    secondary_container: c(0x334B45), on_secondary_container: c(0xCDE8DF),
    tertiary: c(0xAACBE4), tertiary_container: c(0x2A4A5F),
    error: c(0xFFB4AB), error_container: c(0x93000A), on_error_container: c(0xFFDAD6),
    surface: c(0x0E1513), text: c(0xDEE4E0), muted: c(0xBFC9C4), outline: c(0x89938F), outline_variant: c(0x3F4945),
    lowest: c(0x090F0E), low: c(0x171D1B), container: c(0x1B211F), high: c(0x252B29), highest: c(0x303634),
    good: c(0x7BD99F), warn: c(0xF2C14E),
};

/// Dark desktop theme: the XDG portal / GNOME color-scheme, KDE's color scheme, or a "dark" GTK theme.
/// Dark when nothing says otherwise, as on Windows.
pub fn system_dark() -> bool {
    use std::sync::OnceLock;
    static DARK: OnceLock<bool> = OnceLock::new();
    *DARK.get_or_init(|| {
        if let Ok(t) = std::env::var("GTK_THEME") { return t.to_ascii_lowercase().contains("dark"); }
        if let Some(v) = crate::util::output("gsettings", &["get", "org.gnome.desktop.interface", "color-scheme"]) {
            if v.contains("prefer-dark") { return true; }
            if v.contains("prefer-light") { return false; }
        }
        let kde = crate::util::config_home().join("kdeglobals");
        if let Ok(text) = std::fs::read_to_string(kde) {
            // [Colors:Window] BackgroundNormal=r,g,b decides, the scheme name is a hint.
            let mut in_window = false;
            for line in text.lines() {
                let l = line.trim();
                if l.starts_with('[') { in_window = l == "[Colors:Window]"; continue; }
                if in_window { if let Some(rgb) = l.strip_prefix("BackgroundNormal=") {
                    let c: Vec<f32> = rgb.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if c.len() >= 3 { return 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2] < 128.0; }
                } }
            }
            if let Some(name) = text.lines().find_map(|l| l.trim().strip_prefix("ColorScheme=")) { return name.to_ascii_lowercase().contains("dark"); }
        }
        if let Some(v) = crate::util::output("gsettings", &["get", "org.gnome.desktop.interface", "gtk-theme"]) { return v.to_ascii_lowercase().contains("dark"); }
        true
    })
}

pub fn palette(theme: &str) -> Palette {
    match theme { "light" => LIGHT, "dark" => DARK, _ => if system_dark() { DARK } else { LIGHT } }
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.surface; v.window_fill = p.container; v.extreme_bg_color = p.highest; v.faint_bg_color = p.low;
    v.code_bg_color = p.low; v.override_text_color = Some(p.text);
    v.window_corner_radius = CornerRadius::same(28); v.menu_corner_radius = CornerRadius::same(12);
    v.window_stroke = Stroke::NONE; v.window_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if p.dark { 140 } else { 50 }) };
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if p.dark { 120 } else { 40 }) };
    v.selection.bg_fill = p.primary.gamma_multiply(0.35); v.selection.stroke = Stroke::new(1.0, p.primary);
    v.hyperlink_color = p.primary; v.error_fg_color = p.error; v.warn_fg_color = p.warn;
    let r = CornerRadius::same(12);
    for (w, fill) in [(&mut v.widgets.noninteractive, p.surface), (&mut v.widgets.inactive, p.high), (&mut v.widgets.hovered, p.highest), (&mut v.widgets.active, p.highest), (&mut v.widgets.open, p.highest)] {
        w.corner_radius = r; w.bg_fill = fill; w.weak_bg_fill = fill; w.bg_stroke = Stroke::NONE; w.fg_stroke = Stroke::new(1.0, p.text); w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.outline_variant);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.outline);
    v.widgets.active.bg_stroke = Stroke::new(1.5, p.primary);
    v.text_cursor.stroke = Stroke::new(2.0, p.primary);
    v.text_edit_bg_color = Some(p.highest);
    let theme = if p.dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.options_mut(|o| o.theme_preference = if p.dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
    ctx.set_visuals_of(theme, v.clone());
    ctx.set_visuals_of(if p.dark { egui::Theme::Light } else { egui::Theme::Dark }, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0); s.spacing.button_padding = egui::vec2(16.0, 9.0); s.spacing.interact_size.y = 36.0;
        s.spacing.scroll = egui::style::ScrollStyle::thin(); s.spacing.menu_margin = egui::Margin::same(8);
        s.text_styles.insert(egui::TextStyle::Body, FontId::new(14.5, FontFamily::Proportional));
        s.text_styles.insert(egui::TextStyle::Button, FontId::new(14.5, FontFamily::Proportional));
        s.text_styles.insert(egui::TextStyle::Small, FontId::new(12.5, FontFamily::Proportional));
        s.text_styles.insert(egui::TextStyle::Heading, FontId::new(24.0, semibold()));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
        s.interaction.selectable_labels = false;
    });
}

pub fn semibold() -> FontFamily { FontFamily::Name("semibold".into()) }
pub fn icons() -> FontFamily { FontFamily::Name("icons".into()) }

/// The desktop's UI font (+ its semibold) and monospace font, found through fontconfig. Icons come
/// from egui's own icon and emoji fonts, so they look the same everywhere.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let find = |pattern: &str| -> Option<Vec<u8>> {
        let path = crate::util::output("fc-match", &["-f", "%{file}", pattern])?;
        if !(path.ends_with(".ttf") || path.ends_with(".otf") || path.ends_with(".TTF") || path.ends_with(".OTF")) { return None; }
        std::fs::read(path).ok()
    };
    let mut load = |name: &str, patterns: &[&str]| -> bool {
        for p in patterns { if let Some(bytes) = find(p) { fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes))); return true; } }
        false
    };
    let ui = load("ui", &["sans-serif:style=Regular", "sans-serif"]);
    let sb = load("ui-semibold", &["sans-serif:style=SemiBold", "sans-serif:weight=200", "sans-serif:style=Medium", "sans-serif:style=Bold"]);
    let mono = load("mono", &["monospace:style=Regular", "monospace"]);
    let sym = load("symbols", &["Noto Sans Symbols 2", "Noto Sans Symbols"]);
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    if ui { prop.insert(0, "ui".into()); }
    if sym { prop.push("symbols".into()); }
    let mut semibold = vec![]; if sb { semibold.push("ui-semibold".to_string()); } if ui { semibold.push("ui".into()); }
    semibold.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(self::semibold(), semibold);
    // egui's icon font first, then its monochrome emoji, then the text fonts.
    let mut icon: Vec<String> = ["emoji-icon-font", "NotoEmoji-Regular"].iter().map(|s| s.to_string()).filter(|n| fonts.font_data.contains_key(n)).collect();
    icon.extend(fonts.families[&FontFamily::Proportional].iter().filter(|n| !icon.contains(n)).cloned().collect::<Vec<_>>());
    fonts.families.insert(icons(), icon);
    if mono { fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "mono".into()); }
    ctx.set_fonts(fonts);
}

/// Icon glyphs from egui's built-in icon/emoji fonts (the Windows edition used Segoe Fluent Icons).
/// POWER is drawn as a vector (see `widgets::power_glyph`); this glyph is only a fallback.
pub mod icon {
    pub const POWER: &str = "⏻"; pub const HOME: &str = "🏠"; pub const SETTINGS: &str = "⚙"; pub const JOURNAL: &str = "📓";
    pub const ADD: &str = "➕"; pub const REFRESH: &str = "⟳"; pub const SEARCH: &str = "🔍"; pub const STAR: &str = "★";
    pub const MORE: &str = "☰"; pub const DELETE: &str = "🗑"; pub const COPY: &str = "🗐"; pub const CODE: &str = "{}";
    pub const PULSE: &str = "⚡"; pub const DOWN: &str = "⏷"; pub const RIGHT: &str = "⏵"; pub const SHIELD: &str = "🛡";
    pub const GLOBE: &str = "🌐"; pub const DOWNLOAD: &str = "⬇"; pub const UPLOAD: &str = "⬆"; pub const INFO: &str = "ℹ";
    pub const EDIT: &str = "✏"; pub const CLEAR: &str = "🗙"; pub const FOLDER: &str = "🗀"; pub const LINK: &str = "🔗";
    pub const ERROR: &str = "⚠"; pub const CHECK: &str = "✔"; pub const CLOSE: &str = "✖"; pub const CLOCK: &str = "⏱";
    pub const PASTE: &str = "📋"; pub const ROUTE: &str = "🔀"; pub const SPEED: &str = "🚀"; pub const SERVER: &str = "🖧";
    pub const UPDATE: &str = "🔃"; pub const HELP: &str = "？";
}
