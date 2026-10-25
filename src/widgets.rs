//! Hand-drawn icons and the small custom widgets that give the app its look.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Painter, Pos2, Rect, Response, RichText, Sense,
    Shape, Stroke, TextureHandle, Ui, Vec2, pos2, vec2,
};

use crate::api::Track;
use crate::theme::{self, GREEN, HOVER, MUTED, PLACEHOLDER, SELECTED, SUBDUED, TEXT};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Play,
    Pause,
    Previous,
    Next,
    VolumeOff,
    VolumeLow,
    VolumeHigh,
    Search,
    Close,
    Heart,
    Note,
    Clock,
    Disc,
}

/// Draws `icon` to fill `rect`. Icons are drawn in a -0.5..0.5 square.
pub fn paint_icon(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let pt = |x: f32, y: f32| c + vec2(x * s, y * s);
    let line = Stroke::new((s * 0.08).max(1.2), color);
    let fill = |pts: &[(f32, f32)]| {
        p.add(Shape::convex_polygon(
            pts.iter().map(|&(x, y)| pt(x, y)).collect(),
            color,
            Stroke::NONE,
        ));
    };
    let bar = |x0: f32, x1: f32, y0: f32, y1: f32| {
        p.rect_filled(
            Rect::from_min_max(pt(x0, y0), pt(x1, y1)),
            CornerRadius::same((s * 0.03) as u8),
            color,
        );
    };
    let arc = |radius: f32| {
        let points: Vec<Pos2> = (0..=12)
            .map(|i| {
                let a = (-45.0 + 90.0 * i as f32 / 12.0f32).to_radians();
                pt(0.02 + radius * a.cos(), radius * a.sin())
            })
            .collect();
        p.add(Shape::line(points, line));
    };
    let speaker = || {
        bar(-0.42, -0.2, -0.13, 0.13);
        fill(&[(-0.22, -0.13), (0.0, -0.36), (0.0, 0.36), (-0.22, 0.13)]);
    };

    match icon {
        Icon::Play => fill(&[(-0.28, -0.38), (0.38, 0.0), (-0.28, 0.38)]),
        Icon::Pause => {
            bar(-0.3, -0.08, -0.36, 0.36);
            bar(0.08, 0.3, -0.36, 0.36);
        }
        Icon::Previous => {
            bar(-0.36, -0.24, -0.34, 0.34);
            fill(&[(0.34, -0.34), (0.34, 0.34), (-0.22, 0.0)]);
        }
        Icon::Next => {
            bar(0.24, 0.36, -0.34, 0.34);
            fill(&[(-0.34, -0.34), (0.22, 0.0), (-0.34, 0.34)]);
        }
        Icon::VolumeOff => {
            speaker();
            p.line_segment([pt(0.14, -0.14), pt(0.42, 0.14)], line);
            p.line_segment([pt(0.14, 0.14), pt(0.42, -0.14)], line);
        }
        Icon::VolumeLow => {
            speaker();
            arc(0.18);
        }
        Icon::VolumeHigh => {
            speaker();
            arc(0.18);
            arc(0.36);
        }
        Icon::Search => {
            p.circle_stroke(pt(-0.06, -0.06), 0.26 * s, line);
            p.line_segment([pt(0.14, 0.14), pt(0.38, 0.38)], line);
        }
        Icon::Close => {
            p.line_segment([pt(-0.3, -0.3), pt(0.3, 0.3)], line);
            p.line_segment([pt(-0.3, 0.3), pt(0.3, -0.3)], line);
        }
        Icon::Heart => {
            p.circle_filled(pt(-0.18, -0.1), 0.2 * s, color);
            p.circle_filled(pt(0.18, -0.1), 0.2 * s, color);
            fill(&[(-0.375, -0.04), (0.375, -0.04), (0.0, 0.38)]);
        }
        Icon::Note => {
            p.circle_filled(pt(-0.12, 0.22), 0.14 * s, color);
            p.line_segment([pt(0.0, 0.24), pt(0.0, -0.36)], line);
            p.line_segment([pt(0.0, -0.36), pt(0.24, -0.2)], line);
        }
        Icon::Clock => {
            let thin = Stroke::new((s * 0.07).max(1.0), color);
            p.circle_stroke(c, 0.38 * s, thin);
            p.line_segment([c, pt(0.0, -0.22)], thin);
            p.line_segment([c, pt(0.16, 0.08)], thin);
        }
        Icon::Disc => {
            p.circle_stroke(c, 0.38 * s, line);
            p.circle_filled(c, 0.08 * s, color);
        }
    }
}

/// Runs `add_contents` in a child `Ui` placed by `builder` (usually a rect inside
/// space that is already allocated), without moving the parent's layout cursor.
pub fn place<R>(
    ui: &mut Ui,
    builder: egui::UiBuilder,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut child = ui.new_child(builder);
    add_contents(&mut child)
}

/// The app's logo: a green circle with a play symbol.
pub fn paint_logo(p: &Painter, center: Pos2, radius: f32) {
    p.circle_filled(center, radius, GREEN);
    let icon = Rect::from_center_size(center + vec2(radius * 0.06, 0.0), Vec2::splat(radius * 0.9));
    paint_icon(p, icon, Icon::Play, Color32::BLACK);
}

pub fn logo(ui: &mut Ui, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(radius * 2.0), Sense::hover());
    paint_logo(ui.painter(), rect.center(), radius);
}

/// A borderless icon that lights up on hover.
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, enabled: bool) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size + 8.0), sense);
    let color = if !enabled {
        MUTED
    } else if response.hovered() {
        TEXT
    } else {
        SUBDUED
    };
    paint_icon(ui.painter(), rect.shrink(4.0), icon, color);
    response
}

/// The round play/pause button.
pub fn play_button(
    ui: &mut Ui,
    diameter: f32,
    playing: bool,
    fill: Color32,
    enabled: bool,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), sense);
    let grow = if enabled && response.hovered() {
        1.06
    } else {
        1.0
    };
    let fill = if enabled { fill } else { MUTED };
    ui.painter()
        .circle_filled(rect.center(), diameter / 2.0 * grow, fill);
    let icon = if playing { Icon::Pause } else { Icon::Play };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(diameter * 0.38 * grow));
    paint_icon(ui.painter(), icon_rect, icon, Color32::BLACK);
    response
}

pub enum PillStyle {
    /// Green with black text, for the main action.
    Primary,
    /// Outlined, for secondary actions.
    Outline,
}

/// A rounded "pill" button with bold text.
pub fn pill_button(
    ui: &mut Ui,
    text: &str,
    style: PillStyle,
    font_size: f32,
    full_width: bool,
) -> Response {
    let font = theme::bold(font_size);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let mut size = galley.size() + vec2(font_size * 3.0, font_size * 1.4);
    if full_width {
        size.x = ui.available_width();
    }
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let radius = CornerRadius::same(255);
    let text_color = match style {
        PillStyle::Primary => {
            let fill = if hovered {
                Color32::from_rgb(0x3b, 0xe4, 0x77)
            } else {
                GREEN
            };
            ui.painter().rect_filled(rect, radius, fill);
            Color32::BLACK
        }
        PillStyle::Outline => {
            let stroke = if hovered { TEXT } else { MUTED };
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, stroke),
                egui::StrokeKind::Inside,
            );
            TEXT
        }
    };
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, text_color);
    response
}

/// A thin Spotify-style bar slider. `value` is 0..1.
/// Returns the response and the new value while the user is pressing it.
pub fn thin_slider(ui: &mut Ui, value: f32, width: f32, enabled: bool) -> (Response, Option<f32>) {
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 16.0), sense);
    let mut shown = value.clamp(0.0, 1.0);
    let mut new_value = None;
    if enabled
        && (response.is_pointer_button_down_on() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        shown = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        new_value = Some(shown);
    }
    let active = enabled && (response.hovered() || response.dragged() || new_value.is_some());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    let radius = CornerRadius::same(2);
    ui.painter()
        .rect_filled(track, radius, Color32::from_rgb(0x4d, 0x4d, 0x4d));
    let mut filled = track;
    filled.set_right(track.left() + track.width() * shown);
    let fill = match (enabled, active) {
        (false, _) => MUTED,
        (true, true) => GREEN,
        (true, false) => TEXT,
    };
    ui.painter().rect_filled(filled, radius, fill);
    if active {
        ui.painter()
            .circle_filled(pos2(filled.right(), track.center().y), 6.0, TEXT);
    }
    (response, new_value)
}

/// A texture filled by `color_at(x, y)` with x and y in 0..1. Used for gradients.
pub fn gradient_texture(
    ctx: &egui::Context,
    name: &str,
    [w, h]: [usize; 2],
    color_at: impl Fn(f32, f32) -> Color32,
) -> TextureHandle {
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            pixels.push(color_at(
                x as f32 / (w - 1).max(1) as f32,
                y as f32 / (h - 1).max(1) as f32,
            ));
        }
    }
    let image = egui::ColorImage::new([w, h], pixels);
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Square artwork used where there is no cover image.
pub enum Tile<'a> {
    /// Purple-to-teal tile with a heart, like Spotify's Liked Songs.
    Liked(&'a TextureHandle),
    Playlist,
    Album,
    /// A loaded cover image.
    Cover(&'a TextureHandle),
}

pub fn paint_tile(ui: &Ui, rect: Rect, tile: Tile, radius: u8) {
    let corner = CornerRadius::same(radius);
    match tile {
        Tile::Liked(texture) | Tile::Cover(texture) => {
            egui::Image::new((texture.id(), rect.size()))
                .corner_radius(corner)
                .paint_at(ui, rect);
            if matches!(tile, Tile::Liked(_)) {
                let icon = Rect::from_center_size(rect.center(), rect.size() * 0.4);
                paint_icon(ui.painter(), icon, Icon::Heart, TEXT);
            }
        }
        Tile::Playlist | Tile::Album => {
            ui.painter().rect_filled(rect, corner, PLACEHOLDER);
            let icon = if matches!(tile, Tile::Album) {
                Icon::Disc
            } else {
                Icon::Note
            };
            let icon_rect = Rect::from_center_size(rect.center(), rect.size() * 0.42);
            paint_icon(ui.painter(), icon_rect, icon, theme::SUBDUED);
        }
    }
}

/// One entry in the library list on the left (Liked Songs or a playlist).
pub fn library_row(
    ui: &mut Ui,
    tile: Tile,
    title: &str,
    subtitle: &str,
    selected: bool,
    playing: bool,
) -> Response {
    let height = 56.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let bg = if selected {
        Some(SELECTED)
    } else if response.hovered() {
        Some(HOVER)
    } else {
        None
    };
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    }
    let tile_rect = Rect::from_min_size(rect.min + vec2(8.0, 6.0), Vec2::splat(44.0));
    paint_tile(ui, tile_rect, tile, 4);

    let text_rect = Rect::from_min_max(
        pos2(tile_rect.right() + 12.0, rect.top() + 9.0),
        pos2(rect.right() - 8.0, rect.bottom() - 9.0),
    );
    let title_color = if playing { GREEN } else { TEXT };
    two_line_text(ui, text_rect, title, title_color, subtitle);
    response
}

/// A title with a smaller grey line under it, both cut off with "…" if too long.
pub fn two_line_text(ui: &mut Ui, rect: Rect, title: &str, title_color: Color32, subtitle: &str) {
    place(
        ui,
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(
                egui::Label::new(
                    RichText::new(title)
                        .font(theme::regular(15.0))
                        .color(title_color),
                )
                .truncate(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(subtitle)
                        .font(theme::regular(13.0))
                        .color(SUBDUED),
                )
                .truncate(),
            );
        },
    );
}

// ---- Song table ----

pub const TRACK_ROW_HEIGHT: f32 = 52.0;
pub const TRACK_HEADER_HEIGHT: f32 = 36.0;

/// x offset and width of each column: number, title, album, duration.
fn columns(width: f32) -> [(f32, f32); 4] {
    let num = 44.0;
    let dur = 64.0;
    let rest = (width - num - dur).max(120.0);
    let title = rest * 0.58;
    let album = rest - title;
    [
        (0.0, num),
        (num, title),
        (num + title, album),
        (num + title + album, dur),
    ]
}

pub fn track_header(ui: &mut Ui, rect: Rect) {
    let cols = columns(rect.width());
    let p = ui.painter();
    let font = theme::regular(13.0);
    let y = rect.center().y;
    p.text(
        pos2(rect.left() + cols[0].0 + cols[0].1 / 2.0, y),
        egui::Align2::CENTER_CENTER,
        "#",
        font.clone(),
        SUBDUED,
    );
    for (i, text) in [(1, "Title"), (2, "Album")] {
        p.text(
            pos2(rect.left() + cols[i].0 + 8.0, y),
            egui::Align2::LEFT_CENTER,
            text,
            font.clone(),
            SUBDUED,
        );
    }
    let clock = Rect::from_center_size(
        pos2(rect.left() + cols[3].0 + cols[3].1 / 2.0, y),
        Vec2::splat(15.0),
    );
    paint_icon(p, clock, Icon::Clock, SUBDUED);
    p.hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(1.0, HOVER));
}

/// One song row in a table. Returns true when the user wants to play it
/// (double-click on the row, or a click on the number).
pub fn track_row(
    ui: &mut Ui,
    rect: Rect,
    number: usize,
    t: &Track,
    is_current: bool,
    is_playing: bool,
) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(("track", number)), Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(6), HOVER);
    }
    let cols = columns(rect.width());
    let cell = |i: usize| {
        Rect::from_min_size(
            pos2(rect.left() + cols[i].0, rect.top()),
            vec2(cols[i].1, rect.height()),
        )
    };

    // Number, or a play icon on hover, or a note for the song that is playing.
    let num_rect = cell(0);
    let icon_rect = Rect::from_center_size(num_rect.center(), Vec2::splat(14.0));
    if hovered && t.playable {
        paint_icon(ui.painter(), icon_rect, Icon::Play, TEXT);
    } else if is_current && is_playing {
        paint_icon(ui.painter(), icon_rect, Icon::Note, GREEN);
    } else {
        let color = if is_current { GREEN } else { SUBDUED };
        ui.painter().text(
            num_rect.center(),
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            theme::regular(14.0),
            color,
        );
    }

    let dim = !t.playable;
    let title_color = match (dim, is_current) {
        (true, _) => MUTED,
        (false, true) => GREEN,
        (false, false) => TEXT,
    };
    let title_rect = cell(1).shrink2(vec2(8.0, 8.0));
    two_line_text(ui, title_rect, &t.name, title_color, &t.artists);

    let other = if dim { MUTED } else { SUBDUED };
    let album_rect = cell(2).shrink2(vec2(8.0, 0.0));
    place(
        ui,
        egui::UiBuilder::new()
            .max_rect(album_rect)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.add(egui::Label::new(RichText::new(&t.album).color(other)).truncate());
        },
    );
    ui.painter().text(
        cell(3).center(),
        egui::Align2::CENTER_CENTER,
        format_time(t.duration_ms),
        theme::regular(14.0),
        other,
    );

    if !t.playable {
        response.on_hover_text("This song is not available");
        return false;
    }
    let on_number = response
        .interact_pointer_pos()
        .is_some_and(|p| p.x < rect.left() + cols[1].0);
    response.double_clicked() || (response.clicked() && on_number)
}

pub fn format_time(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
