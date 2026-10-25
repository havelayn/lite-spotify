//! Song Roulette: a roulette wheel with twelve songs and one "wild" pocket.

use std::f32::consts::{PI, TAU};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};
use rand::Rng;

use super::{ease_out, sample};
use crate::api::Track;
use crate::theme::{self, muted, subdued};
use crate::widgets::{self, ButtonStyle};

const SONGS: usize = 12;
/// Pocket 0 is the green "wild" pocket: a random song that is not on the wheel.
const POCKETS: usize = SONGS + 1;
const SPIN_SECONDS: f32 = 5.5;

const RED: Color32 = Color32::from_rgb(0xb3, 0x26, 0x2e);
const BLACK: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x24);
const GREEN: Color32 = Color32::from_rgb(0x13, 0x82, 0x4a);
const WOOD: Color32 = Color32::from_rgb(0x4a, 0x2c, 0x17);
const WOOD_LIGHT: Color32 = Color32::from_rgb(0x7a, 0x4b, 0x28);
const GOLD: Color32 = Color32::from_rgb(0xd8, 0xb0, 0x4c);

struct Spin {
    start: f64,
    rotation_from: f32,
    rotation_to: f32,
    ball_from: f32,
    ball_to: f32,
    winner: usize,
}

#[derive(Default)]
pub struct Roulette {
    pockets: Vec<Track>,
    version: u64,
    /// Current turn of the wheel, in radians.
    rotation: f32,
    /// Where the ball rests after a spin: angle and distance from the centre (0..1).
    ball: Option<(f32, f32)>,
    spin: Option<Spin>,
    winner: Option<usize>,
}

fn pocket_color(i: usize) -> Color32 {
    match i {
        0 => GREEN,
        i if i % 2 == 1 => RED,
        _ => BLACK,
    }
}

impl Roulette {
    fn deal(&mut self, pool: &[Track], version: u64) {
        self.pockets = sample(pool, SONGS);
        self.version = version;
        self.winner = None;
        self.ball = None;
    }

    fn start_spin(&mut self, time: f64) {
        let mut rng = rand::rng();
        let winner = rng.random_range(0..POCKETS);
        let step = TAU / POCKETS as f32;
        // Turn the wheel at least four times and stop with the winner at the top.
        let mut rotation_to = -PI / 2.0 - winner as f32 * step;
        while rotation_to < self.rotation + 4.0 * TAU {
            rotation_to += TAU;
        }
        // The ball runs the other way at least seven times and ends at the top too.
        let ball_from = rng.random_range(0.0..TAU);
        let mut ball_to = -PI / 2.0;
        while ball_to > ball_from - 7.0 * TAU {
            ball_to -= TAU;
        }
        self.spin = Some(Spin {
            start: time,
            rotation_from: self.rotation,
            rotation_to,
            ball_from,
            ball_to,
            winner,
        });
        self.winner = None;
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        pool: &[Track],
        version: u64,
    ) -> Option<(Vec<Track>, String)> {
        if self.pockets.is_empty() || self.version != version && self.spin.is_none() {
            self.deal(pool, version);
        }
        let time = ui.input(|i| i.time);
        let mut result = None;

        // Move the wheel and the ball.
        let mut ball = self.ball;
        if let Some(spin) = &self.spin {
            let t = ((time - spin.start) as f32 / SPIN_SECONDS).clamp(0.0, 1.0);
            let e = ease_out(t);
            self.rotation = spin.rotation_from + (spin.rotation_to - spin.rotation_from) * e;
            let angle = spin.ball_from + (spin.ball_to - spin.ball_from) * ease_out(t.powf(0.85));
            // The ball runs on the outer track, then drops into a pocket with a little bounce.
            let drop = ((t - 0.72) / 0.28).clamp(0.0, 1.0);
            let bounce = (drop * PI * 3.0).sin().abs() * (1.0 - drop) * 0.05;
            let radius = 0.9 - (0.9 - 0.72) * drop * drop + bounce;
            ball = Some((angle, radius));
            if t >= 1.0 {
                let winner = spin.winner;
                self.spin = None;
                self.winner = Some(winner);
                self.ball = Some((-PI / 2.0, 0.72));
                result = Some(if winner == 0 {
                    let wild: Vec<Track> = pool
                        .iter()
                        .filter(|t| !self.pockets.iter().any(|p| p.uri == t.uri))
                        .cloned()
                        .collect();
                    let wild = if wild.is_empty() { pool.to_vec() } else { wild };
                    (
                        sample(&wild, 1),
                        "WILD pocket! A song from anywhere in the list".to_string(),
                    )
                } else {
                    (
                        vec![self.pockets[winner - 1].clone()],
                        format!("The ball landed on {winner}"),
                    )
                });
            } else {
                ui.ctx().request_repaint();
            }
        }

        let full = ui.available_rect_before_wrap();
        let list_w = 280.0_f32.min(full.width() * 0.4);
        let size = (full.width() - list_w - 24.0)
            .min(full.height())
            .clamp(180.0, 560.0);
        let wheel_rect = Rect::from_min_size(full.min, vec2(size, size));
        ui.allocate_rect(
            Rect::from_min_size(full.min, vec2(full.width(), size)),
            Sense::hover(),
        );
        self.paint_wheel(ui, wheel_rect, ball);

        // The list of songs on the wheel, and the buttons.
        let list = Rect::from_min_max(
            pos2(wheel_rect.right() + 24.0, full.top()),
            pos2(full.right(), full.top() + size),
        );
        let spinning = self.spin.is_some();
        widgets::place(
            ui,
            egui::UiBuilder::new()
                .max_rect(list)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.horizontal(|ui| {
                    let label = if spinning { "Spinning…" } else { "Spin!" };
                    let spin =
                        widgets::button_sized(ui, label, ButtonStyle::Accent, 16.0, Some(120.0));
                    if spin.clicked() && !spinning {
                        self.start_spin(time);
                        ui.ctx().request_repaint();
                    }
                    if widgets::button(ui, "New songs", ButtonStyle::Ghost, 13.0).clicked()
                        && !spinning
                    {
                        self.deal(pool, version);
                    }
                });
                ui.add_space(10.0);
                ui.label(
                    RichText::new("ON THE WHEEL")
                        .font(theme::bold(12.0))
                        .color(muted()),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        for i in 0..POCKETS {
                            let winner = self.winner == Some(i);
                            ui.horizontal(|ui| {
                                let (dot, _) =
                                    ui.allocate_exact_size(egui::Vec2::splat(22.0), Sense::hover());
                                ui.painter()
                                    .circle_filled(dot.center(), 11.0, pocket_color(i));
                                ui.painter().text(
                                    dot.center(),
                                    egui::Align2::CENTER_CENTER,
                                    i.to_string(),
                                    theme::bold(11.0),
                                    Color32::WHITE,
                                );
                                let (name, color) = if i == 0 {
                                    ("WILD: any song from the list".to_string(), subdued())
                                } else {
                                    let t = &self.pockets[i - 1];
                                    (
                                        format!("{} · {}", t.name, t.artists),
                                        if winner {
                                            theme::accent()
                                        } else {
                                            theme::text()
                                        },
                                    )
                                };
                                let color = if winner { theme::accent() } else { color };
                                ui.add(
                                    egui::Label::new(RichText::new(name).color(color)).truncate(),
                                );
                            });
                        }
                    });
            },
        );
        result
    }

    fn paint_wheel(&self, ui: &Ui, rect: Rect, ball: Option<(f32, f32)>) {
        let p = ui.painter();
        let c = rect.center();
        let r = rect.width() / 2.0 - 14.0;
        let at = |angle: f32, radius: f32| c + vec2(angle.cos(), angle.sin()) * radius * r;

        // Wooden bowl and ball track.
        p.circle_filled(c + vec2(0.0, 6.0), r + 6.0, Color32::from_black_alpha(90));
        p.circle_filled(c, r, WOOD);
        p.circle_stroke(c, r - 2.0, Stroke::new(2.0, GOLD));
        p.circle_filled(c, r * 0.94, Color32::from_rgb(0x22, 0x26, 0x31));

        // Number ring.
        let step = TAU / POCKETS as f32;
        for i in 0..POCKETS {
            let mid = self.rotation + i as f32 * step;
            let (a0, a1) = (mid - step / 2.0, mid + step / 2.0);
            let parts = 6;
            for k in 0..parts {
                let b0 = a0 + (a1 - a0) * k as f32 / parts as f32;
                let b1 = a0 + (a1 - a0) * (k + 1) as f32 / parts as f32;
                p.add(Shape::convex_polygon(
                    vec![at(b0, 0.58), at(b0, 0.84), at(b1, 0.84), at(b1, 0.58)],
                    pocket_color(i),
                    Stroke::NONE,
                ));
            }
            p.line_segment([at(a0, 0.58), at(a0, 0.84)], Stroke::new(1.5, GOLD));
            let label = if i == 0 {
                "W".to_string()
            } else {
                i.to_string()
            };
            p.text(
                at(mid, 0.71),
                egui::Align2::CENTER_CENTER,
                label,
                theme::bold((r * 0.09).clamp(10.0, 20.0)),
                Color32::WHITE,
            );
        }
        p.circle_stroke(c, r * 0.84, Stroke::new(1.5, GOLD));
        p.circle_stroke(c, r * 0.58, Stroke::new(1.5, GOLD));

        // Highlight the winning pocket.
        if let Some(w) = self.winner {
            let mid = self.rotation + w as f32 * step;
            let points: Vec<Pos2> = (0..=10)
                .map(|k| at(mid - step / 2.0 + step * k as f32 / 10.0, 0.86))
                .collect();
            p.add(Shape::line(points, Stroke::new(4.0, theme::accent())));
        }

        // The turning centre.
        p.circle_filled(c, r * 0.56, WOOD);
        p.circle_filled(c, r * 0.4, WOOD_LIGHT);
        p.circle_filled(c, r * 0.24, WOOD);
        for k in 0..4 {
            let a = self.rotation + k as f32 * PI / 2.0;
            p.line_segment([at(a, 0.05), at(a, 0.3)], Stroke::new(4.0, GOLD));
            p.circle_filled(at(a, 0.3), r * 0.03, GOLD);
        }
        p.circle_filled(c, r * 0.07, GOLD);

        // Pointer at the top.
        let tip = at(-PI / 2.0, 0.9);
        p.add(Shape::convex_polygon(
            vec![tip, tip + vec2(-10.0, -22.0), tip + vec2(10.0, -22.0)],
            theme::accent(),
            Stroke::new(1.5, theme::on_accent()),
        ));

        // The ball.
        if let Some((angle, radius)) = ball {
            let pos = at(angle, radius);
            p.circle_filled(
                pos + vec2(1.5, 2.0),
                r * 0.035,
                Color32::from_black_alpha(120),
            );
            p.circle_filled(pos, r * 0.035, Color32::from_rgb(0xf4, 0xf4, 0xf4));
        }
    }
}
