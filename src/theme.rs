//! Colours, fonts and the egui style: a dark look close to Spotify's.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Vec2,
};

/// Behind the panels.
pub const BG: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
/// The rounded panels (library and main area).
pub const CARD: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
/// Text boxes, menus and placeholders.
pub const RAISED: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
pub const HOVER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const SELECTED: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
pub const PLACEHOLDER: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);

pub const TEXT: Color32 = Color32::WHITE;
pub const SUBDUED: Color32 = Color32::from_rgb(0xb3, 0xb3, 0xb3);
pub const MUTED: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x6a);
pub const GREEN: Color32 = Color32::from_rgb(0x1e, 0xd7, 0x60);
pub const ERROR: Color32 = Color32::from_rgb(0xf1, 0x5e, 0x6c);

/// Background colours for the top of the main area, picked per playlist.
pub const HEADER_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x50, 0x38, 0xa0), // purple (Liked Songs)
    Color32::from_rgb(0x1e, 0x5a, 0x7a),
    Color32::from_rgb(0x7a, 0x3a, 0x2c),
    Color32::from_rgb(0x2e, 0x6b, 0x45),
    Color32::from_rgb(0x8a, 0x2c, 0x5a),
    Color32::from_rgb(0x6b, 0x5a, 0x22),
    Color32::from_rgb(0x2c, 0x45, 0x8a),
    Color32::from_rgb(0x5a, 0x5a, 0x5a),
];

/// Font family with the bold weight.
fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // Inter (SIL Open Font License, see assets/fonts), cut down to Latin letters.
    // Other scripts fall back to egui's fonts and then to the system's fonts.
    fonts.font_data.insert(
        "Inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Inter-Bold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Bold.ttf"
        ))),
    );
    let fallbacks = fonts.families[&FontFamily::Proportional].clone();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".into());
    let mut bold_fonts = vec!["Inter-Bold".to_string()];
    bold_fonts.extend(fallbacks);
    fonts.families.insert(bold_family(), bold_fonts);
    ctx.set_fonts(fonts);
}

pub fn setup(ctx: &egui::Context) {
    setup_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = HOVER;
    v.window_stroke = Stroke::NONE;
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = RAISED;
    v.faint_bg_color = RAISED;
    v.code_bg_color = RAISED;
    v.hyperlink_color = GREEN;
    v.selection.bg_fill = Color32::from_rgb(0x1d, 0x6b, 0x38);
    v.selection.stroke = Stroke::new(1.0, GREEN);
    v.text_edit_bg_color = Some(RAISED);

    let radius = CornerRadius::same(6);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, HOVER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, SUBDUED);
    v.widgets.inactive.bg_fill = HOVER;
    v.widgets.inactive.weak_bg_fill = HOVER;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_fill = SELECTED;
    v.widgets.hovered.weak_bg_fill = SELECTED;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.bg_fill = SELECTED;
    v.widgets.active.weak_bg_fill = SELECTED;
    v.widgets.active.bg_stroke = Stroke::new(1.0, TEXT);
    ctx.set_visuals_of(egui::Theme::Dark, v);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.selectable_labels = false;
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        for (text_style, font) in style.text_styles.iter_mut() {
            *font = match text_style {
                egui::TextStyle::Small => regular(12.0),
                egui::TextStyle::Heading => bold(24.0),
                egui::TextStyle::Monospace => FontId::monospace(13.0),
                _ => regular(14.0),
            };
        }
    });
}
