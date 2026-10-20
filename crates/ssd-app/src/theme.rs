//! Colours, fonts and egui styling of the debug panel (the approved design).

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
};
use std::sync::Arc;

/// The window behind everything.
pub const GROUND: Color32 = Color32::from_rgb(0x0E, 0x10, 0x13);
/// Header, panel and tab strip.
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x16, 0x1B);
/// Hairlines between areas.
pub const LINE: Color32 = Color32::from_rgb(0x25, 0x2A, 0x32);
/// Borders of controls.
pub const LINE_STRONG: Color32 = Color32::from_rgb(0x2C, 0x32, 0x3B);
/// Control background.
pub const CONTROL: Color32 = Color32::from_rgb(0x1B, 0x1F, 0x25);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x23, 0x28, 0x30);
pub const TEXT: Color32 = Color32::from_rgb(0xD9, 0xDE, 0xE5);
pub const MUTED: Color32 = Color32::from_rgb(0x8A, 0x94, 0xA3);
pub const MUTED_LIGHT: Color32 = Color32::from_rgb(0x9A, 0xA4, 0xB2);
pub const DISABLED: Color32 = Color32::from_rgb(0x6B, 0x74, 0x82);
pub const DISABLED_LINE: Color32 = Color32::from_rgb(0x22, 0x26, 0x2D);
/// Rust, selection.
pub const ACCENT: Color32 = Color32::from_rgb(0xF2, 0xA3, 0x3A);
pub const ACCENT_FILL: Color32 = Color32::from_rgb(0x2A, 0x20, 0x16);
pub const TOAST_FILL: Color32 = Color32::from_rgb(0x1F, 0x19, 0x13);
pub const TOAST_LINE: Color32 = Color32::from_rgb(0x3A, 0x2A, 0x14);
pub const TOAST_TEXT: Color32 = Color32::from_rgb(0xFF, 0xD3, 0x9A);
/// Verify, frozen bytes, lockstep running: one blue for all of them.
pub const VERIFY: Color32 = Color32::from_rgb(0x7F, 0xB2, 0xF5);
pub const VERIFY_FILL: Color32 = Color32::from_rgb(0x16, 0x27, 0x3D);
/// The outline of a running verification.
pub const VERIFY_LINE: Color32 = Color32::from_rgb(0x1F, 0x30, 0x47);
/// A frozen value's row in the freezes list, and a replay's row while it is verified.
pub const VERIFY_ROW: Color32 = Color32::from_rgb(0x16, 0x21, 0x2F);
pub const RUST_FILL: Color32 = Color32::from_rgb(0x3A, 0x2A, 0x14);
pub const ORIGINAL_FILL: Color32 = Color32::from_rgb(0x3A, 0x40, 0x4A);
/// Bright text: the selected mode of Original, the bytes that changed.
pub const BRIGHT: Color32 = Color32::from_rgb(0xE6, 0xEA, 0xF0);
/// The bars of the frames before the last.
pub const BAR: Color32 = Color32::from_rgb(0x3A, 0x43, 0x50);
/// Behind the bytes of the selected symbol.
pub const SYMBOL_FILL: Color32 = Color32::from_rgb(0x21, 0x1A, 0x12);
/// Border of the hex view.
pub const FIELD_LINE: Color32 = Color32::from_rgb(0x22, 0x26, 0x2D);
/// Secondary text that must stand out from the muted labels: numbers, outcomes, verdicts.
pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xB8, 0xC0, 0xCB);
/// A routine with divergences: its row (fill) and its count (text).
pub const DIVERGE: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x80);
pub const DIVERGE_FILL: Color32 = Color32::from_rgb(0x2E, 0x1A, 0x1C);
/// The label of the debugger's stop banner (its fill and line are the toast's).
pub const BANNER_LABEL: Color32 = Color32::from_rgb(0xC9, 0xA2, 0x6B);

/// IBM Plex Sans semibold: the title.
pub const SEMIBOLD: &str = "semibold";

/// The HD menus' font family: Share Tech Mono, the font of the game's HD text. `DotGothic16` follows it
/// for the two kanji of the cards, which the Latin face does not have.
pub const GAME: &str = "game";

/// `text`, cut at its end with an ellipsis if it is wider than `max_width` points in `font`.
#[must_use]
pub fn fit_text(painter: &egui::Painter, text: &str, font: &FontId, max_width: f32) -> String {
    let width = |s: &str| painter.layout_no_wrap(s.to_owned(), font.clone(), TEXT).size().x;
    if width(text) <= max_width {
        return text.to_owned();
    }
    let mut kept: Vec<char> = text.chars().collect();
    while kept.pop().is_some() {
        let candidate: String = kept.iter().chain(['\u{2026}'].iter()).collect();
        if width(&candidate) <= max_width {
            return candidate;
        }
    }
    "\u{2026}".to_owned()
}

/// Installs the fonts (IBM Plex Sans for the interface, `JetBrains Mono` for numbers and addresses,
/// Share Tech Mono and `DotGothic16` for the HD menus; all under the SIL OFL, licences next to them in
/// `assets/fonts/`), the player's Traditional Chinese font when there is one ([`with_chinese`]), and the style.
pub fn install(ctx: &egui::Context) {
    let mut fonts = fonts();
    if let Some(bytes) = crate::lang::cjk_font() {
        with_chinese(&mut fonts, &bytes);
    }
    ctx.set_fonts(fonts);
    ctx.set_global_style(style());
}

/// The name of the Traditional Chinese font among the fonts, when the player has one (`crate::lang::cjk_font`).
pub const CHINESE: &str = "noto-sans-hk";

/// Adds the Traditional Chinese font `bytes` (loaded at run time, never built in) to the HD menus' family, right
/// after Share Tech Mono: the translation's names and texts take its glyphs (before `DotGothic16`, whose kanji are
/// Japanese forms in pixels), and the debug panel's families take it last.
fn with_chinese(fonts: &mut FontDefinitions, bytes: &[u8]) {
    fonts.font_data.insert(CHINESE.to_owned(), Arc::new(FontData::from_owned(bytes.to_vec())));
    for (family, names) in &mut fonts.families {
        if *family == FontFamily::Name(GAME.into()) {
            names.insert(1.min(names.len()), CHINESE.to_owned());
        } else {
            names.push(CHINESE.to_owned());
        }
    }
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    let data = [
        ("plex", include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf").as_slice()),
        ("plex-semibold", include_bytes!("../../../assets/fonts/IBMPlexSans-SemiBold.ttf")),
        ("jetbrains-mono", include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf")),
        ("share-tech-mono", ssd_hd::font::SHARE_TECH_MONO),
        ("dotgothic16", include_bytes!("../../../assets/fonts/DotGothic16-Regular.ttf")),
    ];
    for (name, bytes) in data {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let families = [
        (FontFamily::Proportional, vec!["plex", "jetbrains-mono"]),
        (FontFamily::Monospace, vec!["jetbrains-mono", "plex"]),
        (FontFamily::Name(SEMIBOLD.into()), vec!["plex-semibold", "plex"]),
        (FontFamily::Name(GAME.into()), vec!["share-tech-mono", "dotgothic16", "plex"]),
    ];
    for (family, names) in families {
        fonts.families.insert(family, names.into_iter().map(str::to_owned).collect());
    }
    fonts
}

/// The font of numbers and addresses.
#[must_use]
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// The font of the title.
#[must_use]
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// The font of the HD menus, at `size` pixels of the screen.
#[must_use]
pub fn game(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(GAME.into()))
}

fn style() -> egui::Style {
    let mut style = egui::Style {
        text_styles: [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(12.0)),
            (TextStyle::Button, FontId::proportional(12.0)),
            (TextStyle::Heading, semibold(13.0)),
            (TextStyle::Monospace, mono(12.0)),
        ]
        .into(),
        ..egui::Style::default()
    };
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 4.0);
    style.spacing.interact_size = egui::vec2(24.0, 24.0);
    style.spacing.combo_width = 80.0;
    style.spacing.scroll.bar_width = 6.0;

    let radius = CornerRadius::same(6);
    let visuals = &mut style.visuals;
    *visuals = egui::Visuals::dark();
    visuals.override_text_color = None;
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = GROUND;
    visuals.faint_bg_color = Color32::from_rgb(0x1A, 0x1E, 0x24);
    visuals.window_stroke = Stroke::new(1.0, LINE_STRONG);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.popup_shadow = egui::Shadow::NONE;
    visuals.window_shadow = egui::Shadow::NONE;
    visuals.selection.bg_fill = ACCENT_FILL;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    let controls = [
        (&mut visuals.widgets.inactive, CONTROL, MUTED_LIGHT),
        (&mut visuals.widgets.hovered, CONTROL_HOVER, TEXT),
        (&mut visuals.widgets.active, CONTROL_HOVER, TEXT),
        (&mut visuals.widgets.open, CONTROL, TEXT),
    ];
    for (widget, fill, text) in controls {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, LINE_STRONG);
        widget.fg_stroke = Stroke::new(1.0, text);
        widget.corner_radius = radius;
        widget.expansion = 0.0;
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_fonts_are_real_font_files() {
        for (name, data) in &fonts().font_data {
            assert!(data.font.len() > 10_000, "{name}");
            // TrueType: version 0x00010000.
            assert_eq!(data.font[..4], [0, 1, 0, 0], "{name}");
        }
    }

    #[test]
    fn every_family_the_panel_uses_has_its_fonts() {
        let fonts = fonts();
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name(SEMIBOLD.into()),
            FontFamily::Name(GAME.into()),
        ] {
            let names = &fonts.families[&family];
            assert!(names.iter().all(|n| fonts.font_data.contains_key(n)), "{family:?}");
        }
    }

    #[test]
    fn a_chinese_font_follows_share_tech_mono_in_the_menus_and_ends_the_other_families() {
        let mut fonts = fonts();
        with_chinese(&mut fonts, b"not checked here");
        assert_eq!(
            fonts.families[&FontFamily::Name(GAME.into())],
            ["share-tech-mono", CHINESE, "dotgothic16", "plex"]
        );
        assert_eq!(
            fonts.families[&FontFamily::Monospace].last().map(String::as_str),
            Some(CHINESE)
        );
        assert!(fonts.font_data.contains_key(CHINESE));
    }

    #[test]
    fn the_style_has_the_design_sizes() {
        let style = style();
        assert_eq!(style.text_styles[&TextStyle::Body].size, 12.0);
        assert_eq!(style.text_styles[&TextStyle::Small].size, 11.0);
        assert_eq!(style.visuals.panel_fill, PANEL);
    }
}
