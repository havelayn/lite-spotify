//! Slot Machine: three reels of songs; the middle row is your line-up.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use rand::Rng;

use super::{ease_out, sample};
use crate::api::Track;
use crate::theme;

const REELS: usize = 3;
const STRIP: usize = 16;
const SECONDS: [f32; REELS] = [1.6, 2.2, 2.8];

const CABINET: Color32 = Color32::from_rgb(0x6b, 0x1f, 0x33);
const GOLD: Color32 = Color32::from_rgb(0xe0, 0xb8, 0x52);
const CREAM: Color32 = Color32::from_rgb(0xf5, 0xef, 0xe1);
const INK: Color32 = Color32::from_rgb(0x22, 0x1c, 0x24);
const INK_SOFT: Color32 = Color32::from_rgb(0x6d, 0x63, 0x6e);

#[derive(Default)]
pub struct Slots {
    strips: Vec<Vec<Track>>,
    version: u64,
    /// How far each reel has turned, in cells.
    offsets: [f32; REELS],
    spin: Option<(f64, [f32; REELS], [f32; REELS])>,
    lever_pulled_at: Option<f64>,
}

impl Slots {
    fn fill(&mut self, pool: &[Track], version: u64) {
        self.strips = (0..REELS).map(|_| sample(pool, STRIP)).collect();
        self.version = version;
        self.offsets = [0.0; REELS];
    }

    fn start(&mut self, time: f64) {
        let mut rng = rand::rng();
        let from = self.offsets;
        let mut to = from;
        for (i, target) in to.iter_mut().enumerate() {
            let len = self.strips[i].len().max(1) as f32;
            *target =
                from[i].round() + len * (2 + i) as f32 + rng.random_range(0..len as usize) as f32;
        }
        self.spin = Some((time, from, to));
        self.lever_pulled_at = Some(time);
    }

    fn middle(&self, reel: usize) -> &Track {
        let strip = &self.strips[reel];
        &strip[(self.offsets[reel].round() as usize) % strip.len()]
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        pool: &[Track],
        version: u64,
    ) -> Option<(Vec<Track>, String)> {
        if (self.strips.is_empty() || self.version != version) && self.spin.is_none() {
            self.fill(pool, version);
        }
        let time = ui.input(|i| i.time);
        let mut result = None;
        if let Some((start, from, to)) = self.spin {
            let elapsed = (time - start) as f32;
            for i in 0..REELS {
                let t = (elapsed / SECONDS[i]).min(1.0);
                self.offsets[i] = from[i] + (to[i] - from[i]) * ease_out(t);
            }
            if elapsed >= SECONDS[REELS - 1] {
                self.offsets = to;
                self.spin = None;
                let line: Vec<Track> = (0..REELS).map(|i| self.middle(i).clone()).collect();
                result = Some((line.clone(), headline(&line)));
            }
        }
        let lever_busy = self.lever_pulled_at.is_some_and(|t| time - t < 0.6);
        if self.spin.is_some() || lever_busy {
            ui.ctx().request_repaint();
        }

        let full = ui.available_rect_before_wrap();
        let width = (full.width() - 60.0).clamp(300.0, 720.0);
        let cell_h = 78.0;
        let window_h = cell_h * 3.0;
        let cabinet = Rect::from_min_size(full.min, vec2(width, window_h + 110.0));
        ui.allocate_rect(
            Rect::from_min_size(full.min, vec2(full.width(), cabinet.height() + 10.0)),
            Sense::hover(),
        );

        let p = ui.painter();
        p.rect_filled(
            cabinet.translate(vec2(0.0, 6.0)),
            CornerRadius::same(16),
            Color32::from_black_alpha(90),
        );
        p.rect_filled(cabinet, CornerRadius::same(16), CABINET);
        p.rect_stroke(
            cabinet.shrink(5.0),
            CornerRadius::same(12),
            Stroke::new(2.0, GOLD),
            egui::StrokeKind::Inside,
        );
        p.text(
            pos2(cabinet.center().x, cabinet.top() + 30.0),
            egui::Align2::CENTER_CENTER,
            "LUCKY SPINS",
            theme::bold(22.0),
            GOLD,
        );

        // Reels.
        let reels =
            Rect::from_min_size(cabinet.min + vec2(24.0, 56.0), vec2(width - 48.0, window_h));
        let reel_w = (reels.width() - 16.0) / REELS as f32;
        for i in 0..REELS {
            let rect = Rect::from_min_size(
                pos2(reels.left() + i as f32 * (reel_w + 8.0), reels.top()),
                vec2(reel_w, window_h),
            );
            self.paint_reel(ui, rect, i, cell_h);
        }
        // Pay line.
        let y = reels.center().y;
        p.line_segment(
            [pos2(reels.left() - 12.0, y), pos2(reels.right() + 12.0, y)],
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(255, 80, 80, 160)),
        );
        for x in [reels.left() - 14.0, reels.right() + 14.0] {
            let dir = if x < reels.center().x { 1.0 } else { -1.0 };
            p.add(Shape::convex_polygon(
                vec![pos2(x, y - 7.0), pos2(x + dir * 10.0, y), pos2(x, y + 7.0)],
                GOLD,
                Stroke::NONE,
            ));
        }

        // Lever on the right: click it (or the button) to spin.
        let lever_base = pos2(cabinet.right() + 18.0, cabinet.center().y + 10.0);
        let pull = self
            .lever_pulled_at
            .map(|t| {
                let x = ((time - t) as f32 / 0.6).min(1.0);
                (x * std::f32::consts::PI).sin()
            })
            .unwrap_or(0.0);
        let knob = lever_base + vec2(14.0, -90.0 + pull * 150.0);
        let lever_rect = Rect::from_min_max(
            pos2(lever_base.x - 6.0, lever_base.y - 110.0),
            pos2(lever_base.x + 34.0, lever_base.y + 70.0),
        );
        let lever = ui
            .interact(lever_rect, ui.scope_id().with("lever"), Sense::click())
            .on_hover_text("Pull the lever");
        p.rect_filled(
            Rect::from_center_size(lever_base, vec2(16.0, 44.0)),
            CornerRadius::same(4),
            Color32::from_rgb(0x40, 0x16, 0x22),
        );
        p.line_segment(
            [lever_base, knob],
            Stroke::new(6.0, Color32::from_rgb(0xc8, 0xcd, 0xd8)),
        );
        p.circle_filled(
            knob,
            13.0,
            if lever.hovered() {
                Color32::from_rgb(0xff, 0x55, 0x55)
            } else {
                Color32::from_rgb(0xe0, 0x30, 0x40)
            },
        );
        p.circle_filled(knob + vec2(-4.0, -4.0), 4.0, Color32::from_white_alpha(120));

        let button = Rect::from_center_size(
            pos2(cabinet.center().x, cabinet.bottom() - 26.0),
            vec2(160.0, 34.0),
        );
        let spin_button = ui.interact(button, ui.scope_id().with("spin"), Sense::click());
        let fill = if spin_button.hovered() {
            theme::accent().gamma_multiply(1.15)
        } else {
            theme::accent()
        };
        p.rect_filled(button, CornerRadius::same(17), fill);
        let label = if self.spin.is_some() {
            "SPINNING…"
        } else {
            "PULL!"
        };
        p.text(
            button.center(),
            egui::Align2::CENTER_CENTER,
            label,
            theme::bold(15.0),
            theme::ON_ACCENT,
        );

        if (spin_button.clicked() || lever.clicked()) && self.spin.is_none() {
            self.start(time);
            ui.ctx().request_repaint();
        }
        result
    }

    fn paint_reel(&self, ui: &Ui, rect: Rect, reel: usize, cell_h: f32) {
        let p = ui.painter().with_clip_rect(rect);
        p.rect_filled(rect, CornerRadius::same(8), CREAM);
        let strip = &self.strips[reel];
        let offset = self.offsets[reel];
        let base = offset.floor();
        let frac = offset - base;
        let len = strip.len() as i64;
        for j in -2..=2 {
            let index = ((base as i64 + j as i64) % len + len) % len;
            let track = &strip[index as usize];
            let center_y = rect.center().y + (j as f32 - frac) * cell_h;
            let cell = Rect::from_center_size(
                pos2(rect.center().x, center_y),
                vec2(rect.width() - 12.0, cell_h - 8.0),
            );
            paint_cell(&p, cell, track);
        }
        // Shade the top and bottom so the reel looks round.
        for (y0, y1, a0, a1) in [
            (rect.top(), rect.top() + cell_h, 150u8, 0u8),
            (rect.bottom() - cell_h, rect.bottom(), 0, 150),
        ] {
            let mut mesh = egui::Mesh::default();
            let shade = |a: u8| Color32::from_black_alpha(a);
            mesh.colored_vertex(pos2(rect.left(), y0), shade(a0));
            mesh.colored_vertex(pos2(rect.right(), y0), shade(a0));
            mesh.colored_vertex(pos2(rect.right(), y1), shade(a1));
            mesh.colored_vertex(pos2(rect.left(), y1), shade(a1));
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            p.add(Shape::mesh(mesh));
        }
        p.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(3.0, GOLD),
            egui::StrokeKind::Inside,
        );
    }
}

fn paint_cell(p: &egui::Painter, cell: Rect, track: &Track) {
    let inner = cell.shrink2(vec2(8.0, 6.0));
    let clip = p.with_clip_rect(inner);
    let title = clip.layout_no_wrap(track.name.clone(), theme::bold(14.0), INK);
    clip.galley(Pos2::new(inner.left(), inner.top() + 4.0), title, INK);
    clip.text(
        pos2(inner.left(), inner.bottom() - 4.0),
        egui::Align2::LEFT_BOTTOM,
        &track.artists,
        theme::regular(12.0),
        INK_SOFT,
    );
}

/// The message for a line-up: matching artists are a win!
fn headline(line: &[Track]) -> String {
    let artist = |t: &Track| t.artists.split(", ").next().unwrap_or("").to_string();
    let a: Vec<String> = line.iter().map(artist).collect();
    if a[0] == a[1] && a[1] == a[2] {
        format!("JACKPOT! Three times {}", a[0])
    } else if a[0] == a[1] || a[1] == a[2] || a[0] == a[2] {
        let same = if a[0] == a[1] || a[0] == a[2] {
            &a[0]
        } else {
            &a[1]
        };
        format!("Two of a kind: {same}! Here's your line-up")
    } else {
        "Your line-up of three songs".to_string()
    }
}
