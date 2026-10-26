//! Hand-drawn icons and the small custom widgets that give the app its look.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Painter, Pos2, Rect, Response, RichText, Sense,
    Shape, Stroke, TextureHandle, Ui, Vec2, pos2, vec2,
};

use crate::api::Track;
use crate::theme::{self, hover, muted, on_accent, raised, subdued};

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
    HeartOutline,
    Note,
    Disc,
    Mic,
    Shuffle,
    Repeat,
    RepeatOne,
    Plus,
    /// A small window inside a big one.
    MiniPlayer,
    /// Arrows pointing outwards.
    Expand,
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
        Icon::HeartOutline => {
            // Two arcs that meet in the middle, and two lines down to the tip.
            arc(-0.18, -0.1, 0.2, 140.0, 334.0);
            arc(0.18, -0.1, 0.2, 206.0, 400.0);
            let side = 140.0f32.to_radians();
            let (x, y) = (0.2 * side.cos(), -0.1 + 0.2 * side.sin());
            p.line_segment([pt(-0.18 + x, y), pt(0.0, 0.36)], line);
            p.line_segment([pt(0.18 - x, y), pt(0.0, 0.36)], line);
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
        Icon::MiniPlayer => {
            p.rect_stroke(
                Rect::from_min_max(pt(-0.42, -0.32), pt(0.42, 0.32)),
                CornerRadius::same((s * 0.06) as u8),
                line,
                egui::StrokeKind::Middle,
            );
            p.rect_filled(
                Rect::from_min_max(pt(0.0, 0.02), pt(0.3, 0.22)),
                CornerRadius::same((s * 0.03) as u8),
                color,
            );
        }
        Icon::Expand => {
            p.line_segment([pt(-0.28, 0.28), pt(0.28, -0.28)], line);
            head(0.26, -0.26, 0.707, -0.707);
            head(-0.26, 0.26, -0.707, 0.707);
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
        muted()
    } else if active {
        theme::accent()
    } else if response.hovered() {
        theme::text()
    } else {
        subdued()
    };
    paint_icon(ui.painter(), rect.shrink(5.0), icon, color);
    if active {
        ui.painter()
            .circle_filled(pos2(rect.center().x, rect.bottom() - 1.0), 1.8, color);
    }
    response
}

/// A heart that is filled in the accent colour when `liked`, an outline otherwise.
pub fn heart_button(ui: &mut Ui, size: f32, liked: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size + 10.0), Sense::click());
    let (icon, color) = match (liked, response.hovered()) {
        (true, _) => (Icon::Heart, theme::accent()),
        (false, true) => (Icon::HeartOutline, theme::text()),
        (false, false) => (Icon::HeartOutline, subdued()),
    };
    paint_icon(ui.painter(), rect.shrink(5.0), icon, color);
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
    let fill = if enabled { theme::accent() } else { muted() };
    ui.painter()
        .circle_filled(rect.center(), diameter / 2.0 * grow, fill);
    let icon = if playing { Icon::Pause } else { Icon::Play };
    let icon_rect = Rect::from_center_size(
        rect.center() + vec2(if playing { 0.0 } else { diameter * 0.03 }, 0.0),
        Vec2::splat(diameter * 0.38 * grow),
    );
    paint_icon(ui.painter(), icon_rect, icon, on_accent());
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
            on_accent()
        }
        ButtonStyle::Ghost => {
            let fill = if hovered { hover() } else { raised() };
            ui.painter().rect(
                rect,
                radius,
                fill,
                Stroke::new(1.0, theme::border()),
                egui::StrokeKind::Inside,
            );
            theme::text()
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
    ui.painter().rect_filled(track, radius, theme::border());
    let mut filled = track;
    filled.set_right(track.left() + track.width() * shown);
    let fill = if enabled { theme::accent() } else { muted() };
    ui.painter().rect_filled(filled, radius, fill);
    if active {
        ui.painter()
            .circle_filled(pos2(filled.right(), track.center().y), 7.0, theme::text());
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
            on_accent(),
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
                .rect_filled(rect, CornerRadius::same(radius), raised());
            let icon_rect = Rect::from_center_size(rect.center(), rect.size() * 0.36);
            paint_icon(ui.painter(), icon_rect, icon, muted());
        }
    }
}

/// One line in a list on the left (Liked Songs, a playlist, a podcast).
pub fn nav_row(ui: &mut Ui, icon: Icon, text: &str, selected: bool, playing: bool) -> Response {
    nav_row_with(ui, icon, None, text, selected, playing)
}

/// Like [`nav_row`], with a cover thumbnail instead of the icon when there is one.
pub fn nav_row_with(
    ui: &mut Ui,
    icon: Icon,
    thumb: Option<&TextureHandle>,
    text: &str,
    selected: bool,
    playing: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme::selected());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top() + vec2(0.0, 7.0), vec2(3.0, 18.0)),
            CornerRadius::same(2),
            theme::accent(),
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), hover());
    }
    let color = if playing {
        theme::accent()
    } else if selected {
        theme::text()
    } else {
        subdued()
    };
    match thumb {
        Some(thumb) => {
            let image = Rect::from_center_size(
                pos2(rect.left() + 20.0, rect.center().y),
                Vec2::splat(24.0),
            );
            egui::Image::new((thumb.id(), image.size()))
                .corner_radius(CornerRadius::same(3))
                .paint_at(ui, image);
        }
        None => {
            let icon_rect = Rect::from_center_size(
                pos2(rect.left() + 20.0, rect.center().y),
                Vec2::splat(15.0),
            );
            paint_icon(ui.painter(), icon_rect, icon, color);
        }
    }
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

/// x offset and width of each column: number, title, artist, album, time
/// (with the heart).
fn columns(width: f32) -> [(f32, f32); 5] {
    let num = 44.0;
    let dur = 84.0;
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

/// A column the song table can be sorted by.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortColumn {
    Title,
    Artist,
    Album,
    Time,
}

/// How a list is sorted: by a column, A to Z if true. `None` keeps the list's own order.
pub type Sort = Option<(SortColumn, bool)>;

const SORT_COLUMNS: [Option<SortColumn>; 5] = [
    None,
    Some(SortColumn::Title),
    Some(SortColumn::Artist),
    Some(SortColumn::Album),
    Some(SortColumn::Time),
];

/// The column titles. `labels` names the artist and album columns, which hold
/// the show and release date for podcast episodes.
pub fn table_header(ui: &Ui, rect: Rect, labels: [&str; 2]) {
    paint_header(ui, rect, labels, None, None);
}

/// Column titles that sort the list when clicked: once for A to Z, again for
/// Z to A, a third time (or a click on #) for the list's own order.
/// Returns the new sorting when it changed.
pub fn sortable_header(ui: &mut Ui, rect: Rect, labels: [&str; 2], sort: Sort) -> Option<Sort> {
    let cols = columns(rect.width());
    let mut hovered = None;
    let mut new_sort = None;
    for (i, column) in SORT_COLUMNS.iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(rect.left() + cols[i].0, rect.top()),
            vec2(cols[i].1, rect.height()),
        );
        let response = ui
            .interact(cell, ui.scope_id().with(("sort", i)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            hovered = Some(i);
        }
        if response.clicked() {
            new_sort = Some(match (column, sort) {
                (None, _) => None,
                (Some(c), Some((s, true))) if *c == s => Some((*c, false)),
                (Some(c), Some((s, false))) if *c == s => None,
                (Some(c), _) => Some((*c, true)),
            });
        }
    }
    paint_header(ui, rect, labels, Some(sort), hovered);
    new_sort
}

fn paint_header(
    ui: &Ui,
    rect: Rect,
    labels: [&str; 2],
    sort: Option<Sort>,
    hovered: Option<usize>,
) {
    let cols = columns(rect.width());
    let p = ui.painter();
    let font = theme::bold(12.0);
    let y = rect.center().y;
    let names = ["#", "TITLE", labels[0], labels[1], "TIME"];
    for (i, name) in names.iter().enumerate() {
        let (x, w) = cols[i];
        let sorted = sort.flatten().filter(|(c, _)| SORT_COLUMNS[i] == Some(*c));
        let color = if sorted.is_some() {
            theme::accent()
        } else if hovered == Some(i) {
            theme::text()
        } else {
            muted()
        };
        let (pos, align) = if i == 0 || i == 4 {
            (
                pos2(rect.left() + x + w / 2.0, y),
                egui::Align2::CENTER_CENTER,
            )
        } else {
            (pos2(rect.left() + x + 8.0, y), egui::Align2::LEFT_CENTER)
        };
        let text = p.text(pos, align, *name, font.clone(), color);
        // A small triangle: up for A to Z, down for Z to A.
        if let Some((_, ascending)) = sorted {
            let c = pos2(text.right() + 8.0, y);
            let d = if ascending { -1.0 } else { 1.0 };
            p.add(Shape::convex_polygon(
                vec![
                    c + vec2(-4.0, -2.0 * d),
                    c + vec2(4.0, -2.0 * d),
                    c + vec2(0.0, 3.0 * d),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }
    p.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, theme::border()),
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
    /// Add to Liked Songs, or remove from it.
    pub like: bool,
    /// Add the song to this playlist.
    pub add_to: Option<String>,
    /// Make a new playlist with the song.
    pub new_playlist: bool,
    pub remove: bool,
}

/// How a song row looks.
pub struct RowState {
    pub is_current: bool,
    pub is_playing: bool,
    pub liked: bool,
    /// The album cover, when thumbnails are turned on and it has loaded.
    pub thumb: Option<TextureHandle>,
}

/// One song row: double-click (or click the number) to play, right-click for a
/// menu, click the heart to like it.
pub fn table_row(
    ui: &mut Ui,
    rect: Rect,
    number: usize,
    t: &Track,
    state: RowState,
    menu: &RowMenu,
) -> RowAction {
    let RowState {
        is_current,
        is_playing,
        liked,
        thumb,
    } = state;
    let response = ui.interact(rect, ui.scope_id().with(("row", number)), Sense::click());
    let hovered = response.hovered();
    let bg = if is_current {
        Some(theme::accent_soft())
    } else if hovered {
        Some(hover())
    } else if number.is_multiple_of(2) {
        Some(theme::surface())
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
        paint_icon(ui.painter(), icon_rect, Icon::Play, theme::text());
    } else if is_current && is_playing {
        paint_icon(ui.painter(), icon_rect, Icon::Note, accent);
    } else {
        ui.painter().text(
            num_rect.center(),
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            theme::regular(13.0),
            if is_current { accent } else { muted() },
        );
    }

    let (title_color, other_color) = match (t.playable, is_current) {
        (false, _) => (muted(), muted()),
        (true, true) => (accent, subdued()),
        (true, false) => (theme::text(), subdued()),
    };
    let texts = [
        (1, &t.name, title_color),
        (2, &t.artists, other_color),
        (3, &t.album, other_color),
    ];
    for (i, text, color) in texts {
        let mut cell = cell(i);
        if i == 1
            && let Some(thumb) = &thumb
        {
            let image = Rect::from_min_size(
                pos2(cell.left() + 6.0, cell.center().y - 11.0),
                Vec2::splat(22.0),
            );
            egui::Image::new((thumb.id(), image.size()))
                .corner_radius(CornerRadius::same(3))
                .paint_at(ui, image);
            cell.set_left(image.right() + 2.0);
        }
        place_in(ui, cell.shrink2(vec2(8.0, 0.0)), |ui| {
            ui.add(egui::Label::new(RichText::new(text.as_str()).color(color)).truncate());
        });
    }
    let time_cell = cell(4);
    ui.painter().text(
        pos2(time_cell.right() - 26.0, time_cell.center().y),
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
    // The heart: always shown for liked songs, as an outline on hover otherwise.
    let heart = Rect::from_center_size(
        pos2(time_cell.left() + 14.0, time_cell.center().y),
        Vec2::splat(22.0),
    );
    let heart_response = ui.interact(heart, ui.scope_id().with(("heart", number)), Sense::click());
    if liked || hovered || heart_response.hovered() {
        let (icon, color) = match (liked, heart_response.hovered()) {
            (true, _) => (Icon::Heart, accent),
            (false, true) => (Icon::HeartOutline, theme::text()),
            (false, false) => (Icon::HeartOutline, muted()),
        };
        paint_icon(ui.painter(), heart.shrink(5.0), icon, color);
    }
    if heart_response
        .on_hover_text(if liked {
            "Remove from Liked Songs"
        } else {
            "Save to Liked Songs"
        })
        .clicked()
    {
        action.like = true;
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
        let like = if liked {
            "Remove from Liked Songs"
        } else {
            "Save to Liked Songs"
        };
        if ui.button(like).clicked() {
            action.like = true;
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
