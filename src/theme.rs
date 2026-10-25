//! Colours, fonts and the egui style: a dark "midnight" look with a choice of
//! accent colours.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Vec2,
};

/// Behind everything.
pub const BG: Color32 = Color32::from_rgb(0x0e, 0x10, 0x16);
/// Panels.
pub const SURFACE: Color32 = Color32::from_rgb(0x16, 0x19, 0x22);
/// Text boxes, every other table row, placeholders.
pub const RAISED: Color32 = Color32::from_rgb(0x1c, 0x20, 0x2b);
pub const HOVER: Color32 = Color32::from_rgb(0x25, 0x2a, 0x38);
pub const SELECTED: Color32 = Color32::from_rgb(0x2c, 0x32, 0x44);
pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x3e);

pub const TEXT: Color32 = Color32::from_rgb(0xe9, 0xeb, 0xf2);
pub const SUBDUED: Color32 = Color32::from_rgb(0x9a, 0xa1, 0xb5);
pub const MUTED: Color32 = Color32::from_rgb(0x5d, 0x64, 0x7a);
/// Text drawn on top of the accent colour.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x14, 0x16, 0x1d);
pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x6b, 0x7a);

/// The accent colours to choose from in Settings. The first is the default.
pub const ACCENTS: [(&str, Color32); 5] = [
    ("Amber", Color32::from_rgb(0xff, 0xb0, 0x2e)),
    ("Coral", Color32::from_rgb(0xff, 0x77, 0x6b)),
    ("Violet", Color32::from_rgb(0xa5, 0x8b, 0xff)),
    ("Teal", Color32::from_rgb(0x2e, 0xd3, 0xb7)),
    ("Sky", Color32::from_rgb(0x5c, 0xb4, 0xff)),
];

static ACCENT: AtomicU32 = AtomicU32::new(0xffb02e);

/// The current accent colour.
pub fn accent() -> Color32 {
    let rgb = ACCENT.load(Ordering::Relaxed);
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// A see-through version of the accent, for highlighted backgrounds.
pub fn accent_soft() -> Color32 {
    let a = accent();
    Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 40)
}

/// Switches the accent colour (by name from [`ACCENTS`]) and restyles the app.
pub fn set_accent(ctx: &egui::Context, name: &str) {
    let color = ACCENTS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or(&ACCENTS[0])
        .1;
    let rgb = (color.r() as u32) << 16 | (color.g() as u32) << 8 | color.b() as u32;
    ACCENT.store(rgb, Ordering::Relaxed);
    apply_visuals(ctx);
}

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

pub fn setup(ctx: &egui::Context, accent_name: &str) {
    setup_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    set_accent(ctx, accent_name);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.selectable_labels = false;
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.combo_width = 220.0;
        for (text_style, font) in style.text_styles.iter_mut() {
            *font = match text_style {
                egui::TextStyle::Small => regular(12.0),
                egui::TextStyle::Heading => bold(22.0),
                egui::TextStyle::Monospace => FontId::monospace(13.0),
                _ => regular(14.0),
            };
        }
    });
}

fn apply_visuals(ctx: &egui::Context) {
    let accent = accent();
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = RAISED;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = RAISED;
    v.faint_bg_color = RAISED;
    v.code_bg_color = RAISED;
    v.hyperlink_color = accent;
    v.selection.bg_fill = accent_soft();
    v.selection.stroke = Stroke::new(1.0, accent);
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
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, SUBDUED);
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.bg_fill = SELECTED;
    v.widgets.active.weak_bg_fill = SELECTED;
    v.widgets.active.bg_stroke = Stroke::new(1.0, accent);
    v.widgets.open.weak_bg_fill = SELECTED;
    ctx.set_visuals_of(egui::Theme::Dark, v);
}
