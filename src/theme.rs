//! Colours, fonts and the egui style: a dark "midnight" look with a choice of
//! accent colours.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Vec2,
};

/// The colours of the dark or the light look.
struct Palette {
    bg: Color32,
    surface: Color32,
    raised: Color32,
    hover: Color32,
    selected: Color32,
    border: Color32,
    text: Color32,
    subdued: Color32,
    muted: Color32,
    error: Color32,
}

const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x0e, 0x10, 0x16),
    surface: Color32::from_rgb(0x16, 0x19, 0x22),
    raised: Color32::from_rgb(0x1c, 0x20, 0x2b),
    hover: Color32::from_rgb(0x25, 0x2a, 0x38),
    selected: Color32::from_rgb(0x2c, 0x32, 0x44),
    border: Color32::from_rgb(0x2a, 0x2f, 0x3e),
    text: Color32::from_rgb(0xe9, 0xeb, 0xf2),
    subdued: Color32::from_rgb(0x9a, 0xa1, 0xb5),
    muted: Color32::from_rgb(0x5d, 0x64, 0x7a),
    error: Color32::from_rgb(0xff, 0x6b, 0x7a),
};

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xf3, 0xf4, 0xf8),
    surface: Color32::from_rgb(0xff, 0xff, 0xff),
    raised: Color32::from_rgb(0xea, 0xec, 0xf2),
    hover: Color32::from_rgb(0xdf, 0xe3, 0xec),
    selected: Color32::from_rgb(0xd6, 0xdb, 0xe8),
    border: Color32::from_rgb(0xd3, 0xd7, 0xe2),
    text: Color32::from_rgb(0x16, 0x19, 0x22),
    subdued: Color32::from_rgb(0x55, 0x5c, 0x70),
    muted: Color32::from_rgb(0x8a, 0x91, 0xa5),
    error: Color32::from_rgb(0xc8, 0x2a, 0x40),
};

static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

fn palette() -> &'static Palette {
    if LIGHT_MODE.load(Ordering::Relaxed) {
        &LIGHT
    } else {
        &DARK
    }
}

pub fn is_light() -> bool {
    LIGHT_MODE.load(Ordering::Relaxed)
}

/// Behind everything.
pub fn bg() -> Color32 {
    palette().bg
}
/// Panels.
pub fn surface() -> Color32 {
    palette().surface
}
/// Text boxes, every other table row, placeholders.
pub fn raised() -> Color32 {
    palette().raised
}
pub fn hover() -> Color32 {
    palette().hover
}
pub fn selected() -> Color32 {
    palette().selected
}
pub fn border() -> Color32 {
    palette().border
}
pub fn text() -> Color32 {
    palette().text
}
pub fn subdued() -> Color32 {
    palette().subdued
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn error() -> Color32 {
    palette().error
}
/// Text drawn on top of the accent colour.
pub fn on_accent() -> Color32 {
    Color32::from_rgb(0x14, 0x16, 0x1d)
}

/// The accent colours to choose from in Settings. The first is the default.
/// "Album" follows the cover of the song that is playing.
pub const ACCENTS: [(&str, Color32); 5] = [
    ("Amber", Color32::from_rgb(0xff, 0xb0, 0x2e)),
    ("Coral", Color32::from_rgb(0xff, 0x77, 0x6b)),
    ("Violet", Color32::from_rgb(0xa5, 0x8b, 0xff)),
    ("Teal", Color32::from_rgb(0x2e, 0xd3, 0xb7)),
    ("Sky", Color32::from_rgb(0x5c, 0xb4, 0xff)),
];
pub const ALBUM_ACCENT: &str = "Album";

static ACCENT: AtomicU32 = AtomicU32::new(0xffb02e);

/// The current accent colour (a little darker in the light look, so text in it
/// stays readable).
pub fn accent() -> Color32 {
    let rgb = ACCENT.load(Ordering::Relaxed);
    let c = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    if is_light() {
        Color32::from_rgb(
            (c.r() as f32 * 0.78) as u8,
            (c.g() as f32 * 0.78) as u8,
            (c.b() as f32 * 0.78) as u8,
        )
    } else {
        c
    }
}

/// A see-through version of the accent, for highlighted backgrounds.
pub fn accent_soft() -> Color32 {
    let a = accent();
    Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 40)
}

/// Switches the accent colour (by name from [`ACCENTS`]) and restyles the app.
/// For "Album", the colour is set with [`set_accent_color`] when a cover loads.
pub fn set_accent(ctx: &egui::Context, name: &str) {
    if let Some((_, color)) = ACCENTS.iter().find(|(n, _)| *n == name) {
        set_accent_color(ctx, *color);
    } else {
        apply_visuals(ctx);
    }
}

pub fn set_accent_color(ctx: &egui::Context, color: Color32) {
    let rgb = (color.r() as u32) << 16 | (color.g() as u32) << 8 | color.b() as u32;
    ACCENT.store(rgb, Ordering::Relaxed);
    apply_visuals(ctx);
}

/// Switches between the dark and the light look.
pub fn set_light(ctx: &egui::Context, light: bool) {
    LIGHT_MODE.store(light, Ordering::Relaxed);
    apply_visuals(ctx);
}

/// A bright, colourful colour that stands for a cover: the most common
/// saturated hue, brightened enough to read on the dark background. `None`
/// for covers that are mostly grey.
pub fn cover_color(rgba: &[u8]) -> Option<Color32> {
    let mut bins = [(0.0f32, [0.0f32; 3]); 24];
    for px in rgba.chunks_exact(4).step_by(3) {
        let [r, g, b] = [px[0], px[1], px[2]].map(|c| c as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        if max < 0.2 || saturation < 0.25 {
            continue;
        }
        let hue = if max == min {
            0.0
        } else if max == r {
            ((g - b) / (max - min)).rem_euclid(6.0)
        } else if max == g {
            (b - r) / (max - min) + 2.0
        } else {
            (r - g) / (max - min) + 4.0
        } / 6.0;
        let weight = saturation * max;
        let bin = &mut bins[((hue * 24.0) as usize).min(23)];
        bin.0 += weight;
        bin.1[0] += r * weight;
        bin.1[1] += g * weight;
        bin.1[2] += b * weight;
    }
    let total: f32 = bins.iter().map(|b| b.0).sum();
    let (weight, sum) = bins.iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    // Grey covers with a few coloured pixels keep the normal accent.
    if *weight < 1.0 || *weight < total * 0.08 {
        return None;
    }
    let [r, g, b] = sum.map(|c| c / weight);
    // Brighten so the brightest channel is near the top, keeping the hue.
    let max = r.max(g).max(b).max(0.01);
    let scale = 0.95 / max;
    let lift = |c: f32| ((c * scale).min(1.0) * 255.0) as u8;
    Some(Color32::from_rgb(lift(r), lift(g), lift(b)))
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

pub fn setup(ctx: &egui::Context, accent_name: &str, light: bool) {
    setup_fonts(ctx);
    // egui's own theme setting stays on "dark"; `set_light` swaps the colours.
    ctx.set_theme(egui::Theme::Dark);
    LIGHT_MODE.store(light, Ordering::Relaxed);
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
    let mut v = if is_light() {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    v.panel_fill = bg();
    v.window_fill = raised();
    v.window_stroke = Stroke::new(1.0, border());
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = raised();
    v.faint_bg_color = raised();
    v.code_bg_color = raised();
    v.hyperlink_color = accent;
    v.selection.bg_fill = accent_soft();
    v.selection.stroke = Stroke::new(1.0, accent);
    v.text_edit_bg_color = Some(raised());

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
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border());
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, subdued());
    v.widgets.inactive.bg_fill = raised();
    v.widgets.inactive.weak_bg_fill = raised();
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, border());
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, text());
    v.widgets.hovered.bg_fill = hover();
    v.widgets.hovered.weak_bg_fill = hover();
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, muted());
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, text());
    v.widgets.active.bg_fill = selected();
    v.widgets.active.weak_bg_fill = selected();
    v.widgets.active.bg_stroke = Stroke::new(1.0, accent);
    v.widgets.open.weak_bg_fill = selected();
    ctx.set_visuals_of(egui::Theme::Dark, v);
}
