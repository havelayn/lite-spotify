//! Claw Machine: steer the claw over the capsules, drop it and hope it holds on.
//!
//! The glass box uses its own coordinates: x from 0 to 1 across, y from 0 to
//! [`HEIGHT`] down, both in units of the box width.

use eframe::egui::{
    self, Color32, CornerRadius, Key, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2, pos2,
    vec2,
};
use rand::Rng;

use super::{PARTY, sample};
use crate::api::Track;
use crate::theme::{self, MUTED, SUBDUED};
use crate::widgets::{self, ButtonStyle};

const HEIGHT: f32 = 0.78;
const RADIUS: f32 = 0.05;
/// Where the claw waits at the top.
const TOP: f32 = 0.08;
/// The prize chute is everything left of this.
const CHUTE: f32 = 0.2;
const CHUTE_X: f32 = 0.1;
/// Where the claw starts, and goes back to after a win.
const HOME_X: f32 = 0.55;
const CAPSULES: usize = 16;

const CABINET: Color32 = Color32::from_rgb(0x2e, 0x24, 0x52);
const GLASS: Color32 = Color32::from_rgb(0x10, 0x13, 0x1c);
const METAL: Color32 = Color32::from_rgb(0xc8, 0xcd, 0xd8);

struct Capsule {
    track: Track,
    pos: Pos2,
    color: Color32,
    /// Falling back to the pile after slipping: start time, from y, to y.
    falling: Option<(f64, f32, f32)>,
}

#[derive(Default, Clone, Copy)]
enum Phase {
    #[default]
    Idle,
    Down {
        start: f64,
        depth: f32,
    },
    Grab {
        start: f64,
        depth: f32,
        target: Option<usize>,
        holds: bool,
    },
    Up {
        start: f64,
        depth: f32,
        target: Option<usize>,
        /// How far up (0..1) the prize gets before it slips, if it slips.
        slip_at: Option<f32>,
    },
    Carry {
        start: f64,
        from_x: f32,
        target: usize,
    },
    Release {
        start: f64,
        target: usize,
    },
    /// Gliding back to the middle after dropping a prize.
    Return {
        start: f64,
    },
}

pub struct Claw {
    capsules: Vec<Capsule>,
    version: u64,
    x: f32,
    phase: Phase,
    last_time: f64,
    message: Option<String>,
}

impl Default for Claw {
    fn default() -> Self {
        Self {
            capsules: Vec::new(),
            version: 0,
            x: HOME_X,
            phase: Phase::Idle,
            last_time: 0.0,
            message: None,
        }
    }
}

/// Where the claw's hub is at a moment in the drop, and how open it is (0..1).
struct ClawPose {
    x: f32,
    y: f32,
    open: f32,
    /// Index of the capsule hanging from the claw.
    holding: Option<usize>,
}

impl Claw {
    fn fill(&mut self, pool: &[Track], version: u64) {
        let mut rng = rand::rng();
        let tracks = sample(pool, CAPSULES);
        let per_row = ((1.0 - CHUTE - 0.04) / (RADIUS * 2.05)).floor() as usize;
        self.capsules = tracks
            .into_iter()
            .enumerate()
            .map(|(i, track)| {
                let row = i / per_row;
                let col = i % per_row;
                let offset = if row % 2 == 1 { RADIUS } else { 0.0 };
                let x = CHUTE
                    + 0.03
                    + RADIUS
                    + col as f32 * RADIUS * 2.05
                    + offset
                    + rng.random_range(-0.008..0.008);
                let y = HEIGHT - 0.015 - RADIUS - row as f32 * RADIUS * 1.72;
                Capsule {
                    track,
                    pos: pos2(x.min(0.97 - RADIUS), y),
                    color: PARTY[rng.random_range(0..PARTY.len())],
                    falling: None,
                }
            })
            .collect();
        self.version = version;
        self.phase = Phase::Idle;
        self.message = None;
    }

    fn busy(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// The capsule the claw would land on at `x`, and how deep the claw goes.
    fn landing(&self, x: f32) -> (Option<usize>, f32) {
        let target = self
            .capsules
            .iter()
            .enumerate()
            .filter(|(_, c)| c.falling.is_none() && (c.pos.x - x).abs() < RADIUS * 1.25)
            .min_by(|a, b| a.1.pos.y.total_cmp(&b.1.pos.y))
            .map(|(i, _)| i);
        let depth = match target {
            Some(i) => self.capsules[i].pos.y - RADIUS - 0.03,
            None => HEIGHT - 0.1,
        };
        (target, depth.max(TOP))
    }

    fn drop_claw(&mut self, time: f64) {
        let (_, depth) = self.landing(self.x);
        self.phase = Phase::Down { start: time, depth };
        self.message = None;
    }

    /// Advances the animation; returns a won song when a prize goes down the chute.
    fn update(&mut self, time: f64) -> Option<Track> {
        let since = |start: f64| (time - start) as f32;
        let mut won = None;
        match self.phase {
            // Return is handled below, together with falling capsules.
            Phase::Idle | Phase::Return { .. } => {}
            Phase::Down { start, depth } => {
                if since(start) >= (depth - TOP) / 0.45 {
                    let (target, _) = self.landing(self.x);
                    let holds = target.is_some_and(|i| {
                        let off = (self.capsules[i].pos.x - self.x).abs();
                        let chance = if off < RADIUS * 0.45 { 0.8 } else { 0.45 };
                        rand::rng().random_bool(chance)
                    });
                    self.phase = Phase::Grab {
                        start: time,
                        depth,
                        target,
                        holds,
                    };
                }
            }
            Phase::Grab {
                start,
                depth,
                target,
                holds,
            } => {
                if since(start) >= 0.5 {
                    let slip_at = (!holds).then(|| rand::rng().random_range(0.15..0.7));
                    self.phase = Phase::Up {
                        start: time,
                        depth,
                        target,
                        slip_at,
                    };
                }
            }
            Phase::Up {
                start,
                depth,
                target,
                slip_at,
            } => {
                let duration = (depth - TOP) / 0.4;
                let t = since(start) / duration;
                if let (Some(i), Some(slip)) = (target, slip_at)
                    && t >= slip
                {
                    // It slipped: the capsule falls back where it was.
                    let y = depth + 0.03 + RADIUS - (depth - TOP) * slip;
                    let home = self.capsules[i].pos.y;
                    self.capsules[i].falling = Some((time, y, home));
                    self.message = Some("So close! It slipped out of the claw.".into());
                    self.phase = Phase::Up {
                        start,
                        depth,
                        target: None,
                        slip_at: None,
                    };
                } else if t >= 1.0 {
                    self.phase = match target {
                        Some(target) => Phase::Carry {
                            start: time,
                            from_x: self.x,
                            target,
                        },
                        None => {
                            if self.message.is_none() {
                                self.message = Some("Missed! Have another go.".into());
                            }
                            Phase::Idle
                        }
                    };
                }
            }
            Phase::Carry {
                start,
                from_x,
                target,
            } => {
                let duration = ((from_x - CHUTE_X).abs() / 0.35).max(0.2);
                let t = (since(start) / duration).min(1.0);
                self.x = from_x + (CHUTE_X - from_x) * t;
                if t >= 1.0 {
                    self.phase = Phase::Release {
                        start: time,
                        target,
                    };
                }
            }
            Phase::Release { start, target } => {
                if since(start) >= 1.0 {
                    let capsule = self.capsules.remove(target);
                    won = Some(capsule.track);
                    self.message = Some("You won a prize!".into());
                    self.phase = Phase::Return { start: time };
                }
            }
        }
        if let Phase::Return { start } = self.phase {
            let t = (since(start) / ((HOME_X - CHUTE_X) / 0.35)).min(1.0);
            self.x = CHUTE_X + (HOME_X - CHUTE_X) * t;
            if t >= 1.0 {
                self.phase = Phase::Idle;
            }
        }
        for c in &mut self.capsules {
            if let Some((start, from, to)) = c.falling {
                let t = since(start);
                let y = from + 1.6 * t * t;
                if y >= to {
                    c.pos.y = to;
                    c.falling = None;
                } else {
                    c.pos.y = y;
                }
            }
        }
        won
    }

    fn pose(&self, time: f64) -> ClawPose {
        let since = |start: f64| (time - start) as f32;
        let x = self.x;
        match self.phase {
            Phase::Idle => ClawPose {
                x,
                y: TOP,
                open: 1.0,
                holding: None,
            },
            Phase::Down { start, depth } => {
                let y = (TOP + 0.45 * since(start)).min(depth);
                ClawPose {
                    x,
                    y,
                    open: 1.0,
                    holding: None,
                }
            }
            Phase::Grab {
                start,
                depth,
                target,
                ..
            } => {
                let open = 1.0 - (since(start) / 0.5).min(1.0);
                let holding = if open < 0.5 { target } else { None };
                ClawPose {
                    x,
                    y: depth,
                    open,
                    holding,
                }
            }
            Phase::Up {
                start,
                depth,
                target,
                ..
            } => {
                let y = (depth - 0.4 * since(start)).max(TOP);
                ClawPose {
                    x,
                    y,
                    open: 0.0,
                    holding: target,
                }
            }
            Phase::Carry { target, .. } => ClawPose {
                x,
                y: TOP,
                open: 0.0,
                holding: Some(target),
            },
            Phase::Return { .. } => ClawPose {
                x,
                y: TOP,
                open: 1.0,
                holding: None,
            },
            Phase::Release { start, target } => {
                let open = (since(start) / 0.3).min(1.0);
                ClawPose {
                    x,
                    y: TOP,
                    open,
                    holding: (open < 0.5).then_some(target),
                }
            }
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        pool: &[Track],
        version: u64,
    ) -> Option<(Vec<Track>, String)> {
        if (self.capsules.is_empty() || self.version != version) && !self.busy() {
            self.fill(pool, version);
        }
        let time = ui.input(|i| i.time);
        let dt = (time - self.last_time).clamp(0.0, 0.1) as f32;
        self.last_time = time;

        let full = ui.available_rect_before_wrap();
        let controls_h = 56.0;
        let width = (full.width() - 32.0)
            .min((full.height() - controls_h - 70.0) / HEIGHT)
            .clamp(220.0, 640.0);
        let cabinet = Rect::from_min_size(full.min, vec2(width + 32.0, width * HEIGHT + 70.0));
        let glass =
            Rect::from_min_size(cabinet.min + vec2(16.0, 54.0), vec2(width, width * HEIGHT));
        ui.allocate_rect(
            Rect::from_min_size(full.min, vec2(full.width(), cabinet.height() + controls_h)),
            Sense::hover(),
        );

        // Steering: hold the arrow buttons or the arrow keys.
        let mut steer = 0.0;
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing {
            ui.input(|i| {
                if i.key_down(Key::ArrowLeft) {
                    steer -= 1.0;
                }
                if i.key_down(Key::ArrowRight) {
                    steer += 1.0;
                }
            });
        }
        let mut drop =
            !typing && ui.input(|i| i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::Enter));

        let controls = Rect::from_min_size(
            pos2(cabinet.left(), cabinet.bottom() + 10.0),
            vec2(cabinet.width(), 40.0),
        );
        widgets::place_in(ui, controls, |ui| {
            let left = widgets::button_sized(ui, "◀", ButtonStyle::Ghost, 16.0, Some(56.0));
            if left.is_pointer_button_down_on() {
                steer -= 1.0;
            }
            if widgets::button_sized(ui, "DROP", ButtonStyle::Accent, 16.0, Some(120.0)).clicked() {
                drop = true;
            }
            let right = widgets::button_sized(ui, "▶", ButtonStyle::Ghost, 16.0, Some(56.0));
            if right.is_pointer_button_down_on() {
                steer += 1.0;
            }
            if widgets::button(ui, "Refill", ButtonStyle::Ghost, 13.0).clicked() && !self.busy() {
                self.fill(pool, version);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                match &self.message {
                    Some(message) => {
                        ui.label(
                            RichText::new(message)
                                .font(theme::bold(14.0))
                                .color(theme::accent()),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new("Keys: ← → to move, ↓ to drop")
                                .small()
                                .color(MUTED),
                        );
                    }
                }
                ui.label(
                    RichText::new(format!("{} prizes left", self.capsules.len()))
                        .small()
                        .color(SUBDUED),
                );
            });
        });

        if !self.busy() {
            if steer != 0.0 {
                self.x = (self.x + steer * 0.5 * dt).clamp(0.08, 0.95);
            }
            if drop && !self.capsules.is_empty() {
                self.drop_claw(time);
            }
        }
        let won = self.update(time);
        let animating =
            self.busy() || steer != 0.0 || self.capsules.iter().any(|c| c.falling.is_some());
        if animating {
            ui.ctx().request_repaint();
        }

        self.paint(ui, cabinet, glass, time);

        won.map(|track| (vec![track], "Prize from the claw machine".to_string()))
    }

    fn paint(&self, ui: &Ui, cabinet: Rect, glass: Rect, time: f64) {
        let p = ui.painter();
        let w = glass.width();
        let to_screen = |pt: Pos2| glass.min + pt.to_vec2() * w;

        // Cabinet and sign.
        p.rect_filled(
            cabinet.translate(vec2(0.0, 6.0)),
            CornerRadius::same(14),
            Color32::from_black_alpha(90),
        );
        p.rect_filled(cabinet, CornerRadius::same(14), CABINET);
        let sign = Rect::from_min_size(
            cabinet.min + vec2(16.0, 10.0),
            vec2(cabinet.width() - 32.0, 34.0),
        );
        p.rect_filled(
            sign,
            CornerRadius::same(8),
            Color32::from_rgb(0x1a, 0x14, 0x33),
        );
        p.text(
            sign.center(),
            egui::Align2::CENTER_CENTER,
            "PRIZE CLAW",
            theme::bold(18.0),
            theme::accent(),
        );
        let bulbs = 12;
        let blink = (time * 4.0) as usize;
        for i in 0..bulbs {
            let x = sign.left() + 10.0 + i as f32 * (sign.width() - 20.0) / (bulbs - 1) as f32;
            let on = !self.busy() || (i + blink).is_multiple_of(2);
            let color = if on {
                theme::accent()
            } else {
                Color32::from_rgb(0x55, 0x48, 0x80)
            };
            p.circle_filled(pos2(x, sign.bottom() + 3.0), 2.5, color);
        }

        // Glass box, chute wall and prize hole.
        p.rect_filled(glass, CornerRadius::same(6), GLASS);
        let hole = Rect::from_min_max(
            to_screen(pos2(0.0, HEIGHT - 0.1)),
            to_screen(pos2(CHUTE, HEIGHT)),
        );
        p.rect_filled(
            hole,
            CornerRadius::same(4),
            Color32::from_rgb(0x05, 0x06, 0x0a),
        );
        p.text(
            hole.center(),
            egui::Align2::CENTER_CENTER,
            "PRIZE",
            theme::bold((w * 0.03).max(10.0)),
            MUTED,
        );
        let wall = Rect::from_min_max(
            to_screen(pos2(CHUTE - 0.012, HEIGHT - 0.16)),
            to_screen(pos2(CHUTE, HEIGHT)),
        );
        p.rect_filled(
            wall,
            CornerRadius::same(2),
            Color32::from_rgb(0x8a, 0x93, 0xa8),
        );

        // Capsules.
        let pose = self.pose(time);
        for (i, c) in self.capsules.iter().enumerate() {
            if pose.holding == Some(i) {
                continue;
            }
            paint_capsule(p, to_screen(c.pos), RADIUS * w, c.color);
        }

        // Rail, cable and claw.
        let rail_y = to_screen(pos2(0.0, 0.03)).y;
        p.line_segment(
            [
                pos2(glass.left() + 6.0, rail_y),
                pos2(glass.right() - 6.0, rail_y),
            ],
            Stroke::new(4.0, METAL),
        );
        let carriage = to_screen(pos2(pose.x, 0.03));
        p.rect_filled(
            Rect::from_center_size(carriage, vec2(w * 0.08, w * 0.03)),
            CornerRadius::same(3),
            METAL,
        );
        let hub = to_screen(pos2(pose.x, pose.y));
        p.line_segment([carriage, hub], Stroke::new(2.0, METAL));
        if let Some(i) = pose.holding {
            let release_fall = match self.phase {
                Phase::Release { start, .. } => (time - start) as f32,
                _ => 0.0,
            };
            let pos = hub
                + vec2(
                    0.0,
                    (RADIUS + 0.03) * w + release_fall * release_fall * w * 2.0,
                );
            paint_capsule(p, pos, RADIUS * w, self.capsules[i].color);
        }
        paint_claw(p, hub, w, pose.open);

        // A shine on the glass.
        let shine = Color32::from_white_alpha(14);
        p.line_segment(
            [
                glass.left_top() + vec2(w * 0.7, 6.0),
                glass.left_top() + vec2(w * 0.45, glass.height() - 6.0),
            ],
            Stroke::new(w * 0.04, shine),
        );
        p.rect_stroke(
            glass,
            CornerRadius::same(6),
            Stroke::new(2.0, Color32::from_rgb(0x55, 0x48, 0x80)),
            egui::StrokeKind::Outside,
        );

        // Where the claw will land.
        if !self.busy() {
            let x = to_screen(pos2(pose.x, 0.0)).x;
            let dash_top = hub.y + w * 0.08;
            let mut y = dash_top;
            while y < glass.bottom() - 6.0 {
                p.line_segment(
                    [pos2(x, y), pos2(x, (y + 6.0).min(glass.bottom() - 6.0))],
                    Stroke::new(1.0, Color32::from_white_alpha(40)),
                );
                y += 12.0;
            }
        }
    }
}

fn paint_capsule(p: &egui::Painter, center: Pos2, r: f32, color: Color32) {
    p.circle_filled(
        center + vec2(0.0, r * 0.12),
        r,
        Color32::from_black_alpha(80),
    );
    p.circle_filled(center, r, color);
    // Lighter top half, like a plastic prize capsule.
    let top: Vec<Pos2> = (0..=16)
        .map(|i| {
            let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 16.0;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(Shape::convex_polygon(
        top,
        Color32::from_white_alpha(70),
        Stroke::NONE,
    ));
    p.line_segment(
        [center - vec2(r, 0.0), center + vec2(r, 0.0)],
        Stroke::new(1.5, Color32::from_black_alpha(60)),
    );
    p.circle_filled(
        center + vec2(-r * 0.35, -r * 0.45),
        r * 0.16,
        Color32::from_white_alpha(160),
    );
    p.text(
        center + vec2(0.0, r * 0.35),
        egui::Align2::CENTER_CENTER,
        "?",
        theme::bold(r * 0.7),
        Color32::from_black_alpha(150),
    );
}

fn paint_claw(p: &egui::Painter, hub: Pos2, w: f32, open: f32) {
    let s = w * 0.035;
    let stroke = Stroke::new((w * 0.008).max(2.0), METAL);
    p.circle_filled(hub, s * 0.7, METAL);
    for side in [-1.0_f32, 1.0] {
        let spread = 0.5 + open * 0.9;
        let elbow = hub + vec2(side * s * (0.6 + spread), s * 0.9);
        let tip = hub + vec2(side * s * (0.25 + spread * 0.55), s * 2.3);
        p.line_segment([hub, elbow], stroke);
        p.line_segment([elbow, tip], stroke);
        p.line_segment([tip, tip + vec2(-side * s * 0.4, 0.0)], stroke);
    }
    p.line_segment([hub, hub + Vec2::new(0.0, s * 1.6)], stroke);
}
