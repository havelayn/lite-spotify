//! Guess the Song: a few seconds of a song from your list, and four titles to
//! choose from. The song stays hidden in the player panel until you guess.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Ui, pos2, vec2};
use rand::Rng;

use super::{FunAction, sample};
use crate::api::Track;
use crate::theme::{self, muted, raised, subdued};
use crate::widgets::{self, ButtonStyle, Icon};

const RIGHT: Color32 = Color32::from_rgb(0x2e, 0xb8, 0x6b);
const WRONG: Color32 = Color32::from_rgb(0xe0, 0x4f, 0x5f);

/// Snippet lengths: a tenth of a second first, then longer and longer.
const LENGTHS_MS: [u32; 4] = [100, 1_000, 3_000, 5_000];
/// Points for a right answer after each length: the quicker, the more.
const POINTS: [u32; 4] = [10, 5, 3, 1];

/// "0.1 seconds", "1 second", "3 seconds".
fn seconds_text(ms: u32) -> String {
    match ms {
        1_000 => "1 second".to_string(),
        ms if ms < 1_000 => format!("0.{} seconds", ms / 100),
        ms => format!("{} seconds", ms / 1000),
    }
}

struct Round {
    choices: Vec<Track>,
    answer: usize,
    start_ms: u32,
    /// Index into `LENGTHS_MS` of the longest snippet heard so far.
    length: usize,
    guessed: Option<usize>,
    /// The snippet has played at least once; guessing is possible from then on.
    heard: bool,
}

#[derive(Default)]
pub struct Quiz {
    round: Option<Round>,
    version: u64,
    number: u32,
    score: u32,
    streak: u32,
    best_streak: u32,
    /// The snippet that is playing: when it started and how long it runs.
    playing: Option<(Instant, u32)>,
    /// Waiting for the player to start the snippet.
    waiting: bool,
    unavailable: bool,
}

impl Quiz {
    fn new_round(&mut self, pool: &[Track], version: u64) {
        let choices = sample(pool, 4);
        let mut rng = rand::rng();
        let answer = rng.random_range(0..choices.len());
        let duration = choices[answer].duration_ms.max(40_000);
        // Somewhere in the middle of the song, where the chorus often is.
        let start_ms = rng.random_range(duration / 5..duration * 3 / 5);
        self.round = Some(Round {
            choices,
            answer,
            start_ms,
            length: 0,
            guessed: None,
            heard: false,
        });
        self.version = version;
        self.number += 1;
        self.playing = None;
        self.waiting = false;
        self.unavailable = false;
    }

    /// The player started the snippet.
    pub fn started(&mut self) {
        if self.waiting {
            self.waiting = false;
            if let Some(round) = &mut self.round {
                round.heard = true;
                self.playing = Some((Instant::now(), LENGTHS_MS[round.length]));
            }
        }
    }

    /// The snippet's song could not be played: deal another one.
    pub fn unavailable(&mut self) {
        self.waiting = false;
        self.playing = None;
        self.unavailable = true;
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        pool: &[Track],
        version: u64,
        actions: &mut Vec<FunAction>,
    ) -> Option<(Vec<Track>, String)> {
        if pool.len() < 4 {
            ui.add_space(20.0);
            ui.label(
                RichText::new("Pick a list with at least four songs to play.").color(subdued()),
            );
            return None;
        }
        if self.round.is_none() || self.version != version || self.unavailable {
            self.new_round(pool, version);
        }
        let playing_progress = self.playing.and_then(|(since, length)| {
            let t = since.elapsed().as_secs_f32() / (length as f32 / 1000.0);
            (t < 1.0).then_some(t)
        });
        if playing_progress.is_some() || self.waiting {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        } else {
            self.playing = None;
        }

        // Score line.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Round {}", self.number))
                    .font(theme::bold(16.0))
                    .color(theme::text()),
            );
            ui.label(RichText::new(format!("·  {} points", self.score)).color(subdued()));
            let streak = if self.streak >= 2 {
                format!("·  {} in a row!", self.streak)
            } else {
                String::new()
            };
            ui.label(RichText::new(streak).color(theme::accent()));
            if self.best_streak >= 2 {
                ui.label(
                    RichText::new(format!("·  best: {} in a row", self.best_streak)).color(muted()),
                );
            }
        });
        ui.label(
            RichText::new(
                "It starts with a tenth of a second! Guess right away for 10 points, after \
                 1 second for 5, after 3 seconds for 3, after 5 seconds for 1.",
            )
            .color(subdued()),
        );
        ui.add_space(12.0);

        let round = self.round.as_mut()?;
        let mut play_length = None;
        let mut picked = None;
        let mut next = false;
        let mut play_song = false;

        // The big listen button with a ring that fills while the snippet plays.
        ui.horizontal(|ui| {
            let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(84.0), Sense::click());
            let center = rect.center();
            let hovered = response.hovered();
            ui.painter()
                .circle_filled(center, if hovered { 40.0 } else { 38.0 }, theme::accent());
            let icon = if playing_progress.is_some() || self.waiting {
                Icon::Note
            } else {
                Icon::Play
            };
            widgets::paint_icon(
                ui.painter(),
                Rect::from_center_size(center + vec2(2.0, 0.0), egui::Vec2::splat(30.0)),
                icon,
                theme::on_accent(),
            );
            if let Some(t) = playing_progress {
                let points: Vec<egui::Pos2> = (0..=48)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2
                            + t * std::f32::consts::TAU * i as f32 / 48.0;
                        center + 44.0 * vec2(a.cos(), a.sin())
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(
                    points,
                    egui::Stroke::new(4.0, theme::text()),
                ));
            }
            if response.on_hover_text("Listen again").clicked() {
                play_length = Some(round.length);
            }
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.add_space(14.0);
                let seconds = seconds_text(LENGTHS_MS[round.length]);
                let status = if self.waiting {
                    "Loading…".to_string()
                } else if playing_progress.is_some() {
                    "Listen…".to_string()
                } else {
                    format!("Press play to hear {seconds}")
                };
                ui.label(
                    RichText::new(status)
                        .font(theme::bold(16.0))
                        .color(theme::text()),
                );
                if round.guessed.is_none() && round.length + 1 < LENGTHS_MS.len() {
                    let next = round.length + 1;
                    let label = format!("Play {}", seconds_text(LENGTHS_MS[next]));
                    if widgets::button(ui, &label, ButtonStyle::Ghost, 13.0)
                        .on_hover_text(format!(
                            "Hear more of the song; a right answer is then worth {} points",
                            POINTS[next]
                        ))
                        .clicked()
                    {
                        play_length = Some(next);
                    }
                }
            });
        });
        ui.add_space(16.0);

        // Four answers in a 2 × 2 grid.
        let full = ui.available_rect_before_wrap();
        let gap = 14.0;
        let w = ((full.width() - gap) / 2.0).clamp(140.0, 340.0);
        let h = 64.0;
        ui.allocate_rect(
            Rect::from_min_size(full.min, vec2(full.width(), 2.0 * h + gap)),
            Sense::hover(),
        );
        for (i, track) in round.choices.iter().enumerate() {
            let (col, row) = (i % 2, i / 2);
            let rect = Rect::from_min_size(
                full.min + vec2(col as f32 * (w + gap), row as f32 * (h + gap)),
                vec2(w, h),
            );
            let response = ui.interact(rect, ui.scope_id().with(("quiz", i)), Sense::click());
            let (fill, stroke) = match round.guessed {
                Some(_) if i == round.answer => (RIGHT.gamma_multiply(0.35), RIGHT),
                Some(g) if g == i => (WRONG.gamma_multiply(0.35), WRONG),
                Some(_) => (raised(), theme::border()),
                None if response.hovered() && round.heard => (theme::hover(), theme::accent()),
                None => (raised(), theme::border()),
            };
            ui.painter().rect(
                rect,
                CornerRadius::same(10),
                fill,
                egui::Stroke::new(1.5, stroke),
                egui::StrokeKind::Inside,
            );
            let letter = pos2(rect.left() + 26.0, rect.center().y);
            ui.painter().circle_filled(letter, 14.0, stroke);
            ui.painter().text(
                letter,
                egui::Align2::CENTER_CENTER,
                ["A", "B", "C", "D"][i],
                theme::bold(13.0),
                theme::on_accent(),
            );
            let text = Rect::from_min_max(
                pos2(rect.left() + 50.0, rect.top() + 10.0),
                rect.max - vec2(10.0, 10.0),
            );
            widgets::place(
                ui,
                egui::UiBuilder::new()
                    .max_rect(text)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(
                        egui::Label::new(
                            RichText::new(&track.name)
                                .font(theme::bold(14.0))
                                .color(theme::text()),
                        )
                        .truncate(),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(&track.artists).color(subdued())).truncate(),
                    );
                },
            );
            if round.guessed.is_none() && round.heard && response.clicked() {
                picked = Some(i);
            }
        }
        ui.add_space(14.0);

        if !round.heard {
            ui.label(RichText::new("Press play first, then pick an answer.").color(muted()));
        }
        if let Some(guess) = round.guessed {
            let (text, color) = if guess == round.answer {
                let points = POINTS[round.length];
                (format!("Right! +{points} points"), RIGHT)
            } else {
                let name = &round.choices[round.answer].name;
                (format!("It was \u{201c}{name}\u{201d}"), WRONG)
            };
            ui.add(
                egui::Label::new(RichText::new(text).font(theme::bold(16.0)).color(color))
                    .truncate(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, "Next song", ButtonStyle::Accent, 14.0).clicked() {
                    next = true;
                }
                if widgets::button(ui, "Play this song", ButtonStyle::Ghost, 14.0).clicked() {
                    play_song = true;
                }
            });
        }

        if let Some(guess) = picked {
            round.guessed = Some(guess);
            if guess == round.answer {
                self.score += POINTS[round.length];
                self.streak += 1;
                self.best_streak = self.best_streak.max(self.streak);
            } else {
                self.streak = 0;
            }
        }
        if let Some(length) = play_length {
            round.length = length;
            self.waiting = true;
            self.playing = None;
            actions.push(FunAction::QuizPlay {
                track: round.choices[round.answer].clone(),
                start_ms: round.start_ms,
                length_ms: LENGTHS_MS[length],
            });
        }
        if play_song {
            let track = round.choices[round.answer].clone();
            return Some((vec![track], "From Guess the Song".to_string()));
        }
        if next {
            self.new_round(pool, version);
            // Start the next snippet right away.
            if let Some(round) = &self.round {
                self.waiting = true;
                actions.push(FunAction::QuizPlay {
                    track: round.choices[round.answer].clone(),
                    start_ms: round.start_ms,
                    length_ms: LENGTHS_MS[0],
                });
            }
        }
        None
    }
}
