//! Hand-drawn icons and the small custom widgets that give the app its look.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Painter, Pos2, Rect, Response, RichText, Sense,
    Shape, Stroke, TextureHandle, Ui, Vec2, pos2, vec2,
};

use crate::api::Track;
use crate::theme::{self, HOVER, MUTED, ON_ACCENT, RAISED, SUBDUED, TEXT};

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

/// Shorthand for [`place`] with a rect and a left-to-right, vertically centred layout.
pub fn place_in<R>(ui: &mut Ui, rect: Rect, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    place(
        ui,
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
        add_contents,
    )
}

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
    Disc,
    Mic,
    Shuffle,
    Repeat,
    RepeatOne,
    Plus,
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
    let arc = |cx: f32, cy: f32, radius: f32, from_deg: f32, to_deg: f32| {
        let points: Vec<Pos2> = (0..=16)
            .map(|i| {
                let a = (from_deg + (to_deg - from_deg) * i as f32 / 16.0).to_radians();
                pt(cx + radius * a.cos(), cy + radius * a.sin())
            })
            .collect();
        p.add(Shape::line(points, line));
    };
    let speaker = || {
        bar(-0.42, -0.2, -0.13, 0.13);
        fill(&[(-0.22, -0.13), (0.0, -0.36), (0.0, 0.36), (-0.22, 0.13)]);
    };
    // Small arrow head pointing along (dx, dy) at (x, y).
    let head = |x: f32, y: f32, dx: f32, dy: f32| {
        let (nx, ny) = (-dy, dx);
        fill(&[
            (x + dx * 0.14, y + dy * 0.14),
            (x + nx * 0.11, y + ny * 0.11),
            (x - nx * 0.11, y - ny * 0.11),
        ]);
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
            arc(0.02, 0.0, 0.18, -45.0, 45.0);
        }
        Icon::VolumeHigh => {
            speaker();
            arc(0.02, 0.0, 0.18, -45.0, 45.0);
            arc(0.02, 0.0, 0.36, -45.0, 45.0);
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
        Icon::Disc => {
            p.circle_stroke(c, 0.38 * s, line);
            p.circle_filled(c, 0.08 * s, color);
        }
        Icon::Mic => {
            p.rect_filled(
                Rect::from_min_max(pt(-0.13, -0.42), pt(0.13, 0.08)),
                CornerRadius::same((s * 0.13) as u8),
                color,
            );
            arc(0.0, -0.04, 0.24, 0.0, 180.0);
            p.line_segment([pt(0.0, 0.2), pt(0.0, 0.4)], line);
            p.line_segment([pt(-0.16, 0.4), pt(0.16, 0.4)], line);
        }
        Icon::Shuffle => {
            p.line_segment([pt(-0.4, -0.22), pt(-0.15, -0.22)], line);
            p.line_segment([pt(-0.15, -0.22), pt(0.15, 0.22)], line);
            p.line_segment([pt(0.15, 0.22), pt(0.3, 0.22)], line);
            p.line_segment([pt(-0.4, 0.22), pt(-0.15, 0.22)], line);
            p.line_segment([pt(-0.15, 0.22), pt(0.15, -0.22)], line);
            p.line_segment([pt(0.15, -0.22), pt(0.3, -0.22)], line);
            head(0.3, -0.22, 1.0, 0.0);
            head(0.3, 0.22, 1.0, 0.0);
        }
        Icon::Repeat | Icon::RepeatOne => {
            // A loop that is open at the top right, with an arrow head at its end.
            arc(0.0, 0.0, 0.34, -50.0, 250.0);
            let end = 250.0f32.to_radians();
            let (ex, ey) = (0.34 * end.cos(), 0.34 * end.sin());
            // The loop runs clockwise towards its end; the head points along it.
            head(ex, ey, -end.sin(), end.cos());
            if icon == Icon::RepeatOne {
                p.text(
                    c,
                    egui::Align2::CENTER_CENTER,
                    "1",
                    theme::bold(s * 0.42),
                    color,
                );
            }
        }
        Icon::Plus => {
            p.line_segment([pt(-0.32, 0.0), pt(0.32, 0.0)], line);
            p.line_segment([pt(0.0, -0.32), pt(0.0, 0.32)], line);
        }
    }
}

/// A borderless icon that lights up on hover. `active` draws it in the accent
/// colour with a small dot underneath (for shuffle and repeat).
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, enabled: bool, active: bool) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size + 10.0), sense);
    let color = if !enabled {
        MUTED
    } else if active {
        theme::accent()
    } else if response.hovered() {
        TEXT
    } else {
        SUBDUED
    };
    paint_icon(ui.painter(), rect.shrink(5.0), icon, color);
    if active {
        ui.painter()
            .circle_filled(pos2(rect.center().x, rect.bottom() - 1.0), 1.8, color);
    }
    response
}

/// The round play/pause button.
pub fn play_button(ui: &mut Ui, diameter: f32, playing: bool, enabled: bool) -> Response {
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
    let fill = if enabled { theme::accent() } else { MUTED };
    ui.painter()
        .circle_filled(rect.center(), diameter / 2.0 * grow, fill);
    let icon = if playing { Icon::Pause } else { Icon::Play };
    let icon_rect = Rect::from_center_size(
        rect.center() + vec2(if playing { 0.0 } else { diameter * 0.03 }, 0.0),
        Vec2::splat(diameter * 0.38 * grow),
    );
    paint_icon(ui.painter(), icon_rect, icon, ON_ACCENT);
    response
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonStyle {
    /// Filled with the accent colour, for the main action.
    Accent,
    /// Outlined, for other actions.
    Ghost,
}

/// A rounded button with bold text.
pub fn button(ui: &mut Ui, text: &str, style: ButtonStyle, font_size: f32) -> Response {
    button_sized(ui, text, style, font_size, None)
}

/// Like [`button`], with a fixed width.
pub fn button_sized(
    ui: &mut Ui,
    text: &str,
    style: ButtonStyle,
    font_size: f32,
    width: Option<f32>,
) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        theme::bold(font_size),
        Color32::PLACEHOLDER,
    );
    let mut size = galley.size() + vec2(font_size * 2.0, font_size * 1.1);
    if let Some(w) = width {
        size.x = w;
    }
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let radius = CornerRadius::same(6);
    let text_color = match style {
        ButtonStyle::Accent => {
            let fill = if hovered {
                theme::accent().gamma_multiply(1.15)
            } else {
                theme::accent()
            };
            ui.painter().rect_filled(rect, radius, fill);
            ON_ACCENT
        }
        ButtonStyle::Ghost => {
            let fill = if hovered { HOVER } else { RAISED };
            ui.painter().rect(
                rect,
                radius,
                fill,
                Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
            TEXT
        }
    };
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, text_color);
    response
}

/// A thin bar slider. `value` is 0..1. Returns the response and the new value
/// while the user is pressing it.
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
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 5.0));
    let radius = CornerRadius::same(3);
    ui.painter().rect_filled(track, radius, theme::BORDER);
    let mut filled = track;
    filled.set_right(track.left() + track.width() * shown);
    let fill = if enabled { theme::accent() } else { MUTED };
    ui.painter().rect_filled(filled, radius, fill);
    if active {
        ui.painter()
            .circle_filled(pos2(filled.right(), track.center().y), 7.0, TEXT);
    }
    (response, new_value)
}

/// The app's logo: an accent-coloured rounded square with equaliser bars.
pub fn paint_logo(p: &Painter, rect: Rect) {
    let s = rect.width();
    p.rect_filled(rect, CornerRadius::same((s * 0.25) as u8), theme::accent());
    let heights = [0.35, 0.6, 0.45, 0.7];
    let bar_w = s * 0.11;
    let gap = s * 0.06;
    let total = heights.len() as f32 * bar_w + (heights.len() - 1) as f32 * gap;
    let mut x = rect.center().x - total / 2.0;
    let bottom = rect.bottom() - s * 0.2;
    for h in heights {
        p.rect_filled(
            Rect::from_min_max(pos2(x, bottom - s * h * 0.75), pos2(x + bar_w, bottom)),
            CornerRadius::same((bar_w / 2.0) as u8),
            ON_ACCENT,
        );
        x += bar_w + gap;
    }
}

pub fn logo(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_logo(ui.painter(), rect);
}

/// Album art, or a placeholder with an icon when there is none.
pub fn paint_cover(ui: &Ui, rect: Rect, texture: Option<&TextureHandle>, icon: Icon, radius: u8) {
    match texture {
        Some(texture) => {
            egui::Image::new((texture.id(), rect.size()))
                .corner_radius(CornerRadius::same(radius))
                .paint_at(ui, rect);
        }
        None => {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius), RAISED);
            let icon_rect = Rect::from_center_size(rect.center(), rect.size() * 0.36);
            paint_icon(ui.painter(), icon_rect, icon, MUTED);
        }
    }
}

/// One line in a list on the left (Liked Songs, a playlist, a podcast).
pub fn nav_row(ui: &mut Ui, icon: Icon, text: &str, selected: bool, playing: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme::SELECTED);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top() + vec2(0.0, 7.0), vec2(3.0, 18.0)),
            CornerRadius::same(2),
            theme::accent(),
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), HOVER);
    }
    let color = if playing {
        theme::accent()
    } else if selected {
        TEXT
    } else {
        SUBDUED
    };
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(15.0));
    paint_icon(ui.painter(), icon_rect, icon, color);
    let text_rect = Rect::from_min_max(
        pos2(rect.left() + 36.0, rect.top()),
        pos2(rect.right() - 6.0, rect.bottom()),
    );
    place_in(ui, text_rect, |ui| {
        ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
    });
    response
}

// ---- Song table ----

pub const ROW_HEIGHT: f32 = 30.0;
pub const HEADER_HEIGHT: f32 = 28.0;

/// x offset and width of each column: number, title, artist, album, time.
fn columns(width: f32) -> [(f32, f32); 5] {
    let num = 44.0;
    let dur = 60.0;
    let rest = (width - num - dur).max(150.0);
    let title = rest * 0.4;
    let artist = rest * 0.3;
    let album = rest - title - artist;
    [
        (0.0, num),
        (num, title),
        (num + title, artist),
        (num + title + artist, album),
        (num + title + artist + album, dur),
    ]
}

/// The column titles. `labels` names the artist and album columns, which hold
/// the show and release date for podcast episodes.
pub fn table_header(ui: &Ui, rect: Rect, labels: [&str; 2]) {
    let cols = columns(rect.width());
    let p = ui.painter();
    let font = theme::bold(12.0);
    let y = rect.center().y;
    let names = ["#", "TITLE", labels[0], labels[1], "TIME"];
    for (i, name) in names.iter().enumerate() {
        let (x, w) = cols[i];
        let (pos, align) = if i == 0 || i == 4 {
            (
                pos2(rect.left() + x + w / 2.0, y),
                egui::Align2::CENTER_CENTER,
            )
        } else {
            (pos2(rect.left() + x + 8.0, y), egui::Align2::LEFT_CENTER)
        };
        p.text(pos, align, *name, font.clone(), MUTED);
    }
    p.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, theme::BORDER),
    );
}

/// What the right-click menu of a song row offers besides Play and Add to queue.
#[derive(Default)]
pub struct RowMenu {
    /// The user's own playlists (id, name), for "Add to playlist".
    pub playlists: Vec<(String, String)>,
    /// Show "Remove from this playlist".
    pub can_remove: bool,
}

/// What the user did with a row.
#[derive(Default)]
pub struct RowAction {
    pub play: bool,
    pub queue: bool,
    /// Add the song to this playlist.
    pub add_to: Option<String>,
    /// Make a new playlist with the song.
    pub new_playlist: bool,
    pub remove: bool,
}

/// One song row: double-click (or click the number) to play, right-click for a menu.
pub fn table_row(
    ui: &mut Ui,
    rect: Rect,
    number: usize,
    t: &Track,
    is_current: bool,
    is_playing: bool,
    menu: &RowMenu,
) -> RowAction {
    let response = ui.interact(rect, ui.scope_id().with(("row", number)), Sense::click());
    let hovered = response.hovered();
    let bg = if is_current {
        Some(theme::accent_soft())
    } else if hovered {
        Some(HOVER)
    } else if number.is_multiple_of(2) {
        Some(theme::SURFACE)
    } else {
        None
    };
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    }
    let cols = columns(rect.width());
    let cell = |i: usize| {
        Rect::from_min_size(
            pos2(rect.left() + cols[i].0, rect.top()),
            vec2(cols[i].1, rect.height()),
        )
    };

    let accent = theme::accent();
    let num_rect = cell(0);
    let icon_rect = Rect::from_center_size(num_rect.center(), Vec2::splat(12.0));
    if hovered && t.playable {
        paint_icon(ui.painter(), icon_rect, Icon::Play, TEXT);
    } else if is_current && is_playing {
        paint_icon(ui.painter(), icon_rect, Icon::Note, accent);
    } else {
        ui.painter().text(
            num_rect.center(),
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            theme::regular(13.0),
            if is_current { accent } else { MUTED },
        );
    }

    let (title_color, other_color) = match (t.playable, is_current) {
        (false, _) => (MUTED, MUTED),
        (true, true) => (accent, SUBDUED),
        (true, false) => (TEXT, SUBDUED),
    };
    let texts = [
        (1, &t.name, title_color),
        (2, &t.artists, other_color),
        (3, &t.album, other_color),
    ];
    for (i, text, color) in texts {
        place_in(ui, cell(i).shrink2(vec2(8.0, 0.0)), |ui| {
            ui.add(egui::Label::new(RichText::new(text.as_str()).color(color)).truncate());
        });
    }
    ui.painter().text(
        cell(4).center(),
        egui::Align2::CENTER_CENTER,
        format_time(t.duration_ms),
        theme::regular(13.0),
        other_color,
    );

    let mut action = RowAction::default();
    if !t.playable {
        response.on_hover_text("Not available");
        return action;
    }
    let on_number = response
        .interact_pointer_pos()
        .is_some_and(|p| p.x < rect.left() + cols[1].0);
    action.play = response.double_clicked() || (response.clicked() && on_number);
    response.context_menu(|ui| {
        if ui.button("Play").clicked() {
            action.play = true;
        }
        if ui.button("Add to queue").clicked() {
            action.queue = true;
        }
        if let Some(target) = playlist_menu(ui, &menu.playlists) {
            match target {
                Some(id) => action.add_to = Some(id),
                None => action.new_playlist = true,
            }
        }
        if menu.can_remove {
            ui.separator();
            if ui.button("Remove from this playlist").clicked() {
                action.remove = true;
            }
        }
    });
    action
}

/// An "Add to playlist" submenu. Returns `Some(Some(id))` for a playlist and
/// `Some(None)` for "New playlist…".
pub fn playlist_menu(ui: &mut Ui, playlists: &[(String, String)]) -> Option<Option<String>> {
    let mut picked = None;
    ui.menu_button("Add to playlist", |ui| {
        if ui.button("New playlist…").clicked() {
            picked = Some(None);
        }
        if !playlists.is_empty() {
            ui.separator();
        }
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for (id, name) in playlists {
                    if ui.button(name.as_str()).clicked() {
                        picked = Some(Some(id.clone()));
                    }
                }
            });
    });
    picked
}

pub fn format_time(ms: u32) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
