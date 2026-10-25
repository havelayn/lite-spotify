//! The smaller pickers: Mystery Cards, This or That, Time Machine, Rediscover
//! and Surprise Mix.

use std::collections::BTreeMap;
use std::f32::consts::PI;

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use rand::seq::SliceRandom;

use super::{PARTY, sample, song_card};
use crate::api::Track;
use crate::theme::{self, muted, raised, subdued};
use crate::widgets::{self, ButtonStyle};

const CARD_COUNT: usize = 6;
const FLIP_SECONDS: f32 = 0.5;

/// `pool` songs that pass `keep`, shuffled, at most `max`.
fn shuffled(pool: &[Track], max: usize, keep: impl Fn(&Track) -> bool) -> Vec<Track> {
    let mut songs: Vec<Track> = pool.iter().filter(|t| keep(t)).cloned().collect();
    songs.shuffle(&mut rand::rng());
    songs.truncate(max);
    songs
}

// ---- Mystery Cards ----

#[derive(Default)]
pub struct Cards {
    hand: Vec<Track>,
    version: u64,
    /// The card the user turned over, and when.
    flipped: Option<(usize, f64)>,
    reported: bool,
}

impl Cards {
    fn deal(&mut self, pool: &[Track], version: u64) {
        self.hand = sample(pool, CARD_COUNT);
        self.version = version;
        self.flipped = None;
        self.reported = false;
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        pool: &[Track],
        version: u64,
    ) -> Option<(Vec<Track>, String)> {
        let busy = self.flipped.is_some() && !self.reported;
        if (self.hand.is_empty() || self.version != version) && !busy {
            self.deal(pool, version);
        }
        let time = ui.input(|i| i.time);
        let mut deal = false;
        ui.horizontal(|ui| {
            deal = widgets::button(ui, "Deal new cards", ButtonStyle::Ghost, 13.0).clicked();
            if self.reported {
                ui.label(RichText::new("The others are what you could have had.").color(muted()));
            }
        });
        if deal {
            self.deal(pool, version);
        }
        ui.add_space(12.0);

        let full = ui.available_rect_before_wrap();
        let n = self.hand.len();
        let per_row = if full.width() > 6.0 * 120.0 { n } else { 3 };
        let gap = 14.0;
        let card_w = ((full.width() - gap * (per_row - 1) as f32) / per_row as f32).min(170.0);
        let card_h = card_w * 1.4;
        let rows = n.div_ceil(per_row);
        ui.allocate_rect(
            Rect::from_min_size(full.min, vec2(full.width(), rows as f32 * (card_h + gap))),
            Sense::hover(),
        );

        let mut result = None;
        let mut animating = false;
        for i in 0..n {
            let (row, col) = (i / per_row, i % per_row);
            let mut rect = Rect::from_min_size(
                full.min
                    + vec2(
                        col as f32 * (card_w + gap),
                        row as f32 * (card_h + gap) + 8.0,
                    ),
                vec2(card_w, card_h),
            );
            let response = ui.interact(rect, ui.scope_id().with(("card", i)), Sense::click());
            if self.flipped.is_none() && response.hovered() {
                rect = rect.translate(vec2(0.0, -8.0));
            }
            if response.clicked() && self.flipped.is_none() {
                self.flipped = Some((i, time));
            }

            // Turning over: the chosen card first, the rest a little later.
            let turn = match self.flipped {
                Some((chosen, start)) => {
                    let delay = if chosen == i {
                        0.0
                    } else {
                        0.9 + i as f64 * 0.08
                    };
                    ((time - start - delay) as f32 / FLIP_SECONDS).clamp(0.0, 1.0)
                }
                None => 0.0,
            };
            if self.flipped.is_some() && turn < 1.0 {
                animating = true;
            }
            let squeeze = (turn * PI).cos().abs();
            let shown = Rect::from_center_size(
                rect.center(),
                vec2(rect.width() * squeeze.max(0.02), rect.height()),
            );
            let chosen = self.flipped.is_some_and(|(c, _)| c == i);
            if turn < 0.5 {
                paint_card_back(ui, shown, i);
            } else {
                let fill = if chosen { raised() } else { theme::surface() };
                let stroke = if chosen {
                    theme::accent()
                } else {
                    theme::border()
                };
                song_card(ui, shown, &self.hand[i], fill, stroke);
                if !chosen {
                    ui.painter().rect_filled(
                        shown,
                        CornerRadius::same(8),
                        Color32::from_black_alpha(90),
                    );
                }
            }
            if chosen && turn >= 1.0 && !self.reported {
                self.reported = true;
                result = Some((
                    vec![self.hand[i].clone()],
                    "You turned over this card".to_string(),
                ));
            }
        }
        if animating {
            ui.ctx().request_repaint();
        }
        result
    }
}

fn paint_card_back(ui: &Ui, rect: Rect, i: usize) {
    let p = ui.painter();
    let color = PARTY[i % PARTY.len()];
    p.rect_filled(
        rect.translate(vec2(0.0, 4.0)),
        CornerRadius::same(10),
        Color32::from_black_alpha(80),
    );
    p.rect_filled(
        rect,
        CornerRadius::same(10),
        Color32::from_rgb(0x24, 0x22, 0x3a),
    );
    let inner = rect.shrink(8.0);
    p.rect_stroke(
        inner,
        CornerRadius::same(6),
        Stroke::new(2.0, color),
        egui::StrokeKind::Inside,
    );
    if inner.width() > 20.0 {
        let clip = p.with_clip_rect(inner);
        let mut x = inner.left() - inner.height();
        while x < inner.right() {
            clip.line_segment(
                [
                    pos2(x, inner.bottom()),
                    pos2(x + inner.height(), inner.top()),
                ],
                Stroke::new(1.0, color.gamma_multiply(0.35)),
            );
            x += 12.0;
        }
        p.circle_filled(
            rect.center(),
            inner.width().min(inner.height()) * 0.22,
            color,
        );
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "?",
            theme::bold(inner.width() * 0.3),
            theme::on_accent(),
        );
    }
}

// ---- This or That ----

#[derive(Default)]
pub struct Duel {
    /// Songs still in the running this round.
    round: Vec<Track>,
    /// Songs that won their match this round.
    winners: Vec<Track>,
    /// Which match of the round is being played.
    index: usize,
    round_number: usize,
    version: u64,
    finished: bool,
}

impl Duel {
    fn start(&mut self, pool: &[Track], version: u64) {
        self.round = sample(pool, 8.min(pool.len() / 2 * 2).max(2));
        self.winners.clear();
        self.index = 0;
        self.round_number = 1;
        self.version = version;
        self.finished = false;
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        pool: &[Track],
        version: u64,
    ) -> Option<(Vec<Track>, String)> {
        if self.round.is_empty() || self.version != version {
            self.start(pool, version);
        }
        ui.horizontal(|ui| {
            if self.finished {
                ui.label(
                    RichText::new("We have a winner!")
                        .font(theme::bold(16.0))
                        .color(theme::accent()),
                );
            } else {
                let matches = self.round.len() / 2;
                let stage = match self.round.len() {
                    2 => "Final".to_string(),
                    4 => "Semi-final".to_string(),
                    _ => format!("Round {}", self.round_number),
                };
                ui.label(
                    RichText::new(format!("{stage} · match {} of {matches}", self.index + 1))
                        .font(theme::bold(16.0))
                        .color(theme::text()),
                );
            }
            if widgets::button(ui, "Start over", ButtonStyle::Ghost, 13.0).clicked() {
                self.start(pool, version);
            }
        });
        ui.label(RichText::new("Click the song you'd rather hear right now.").color(subdued()));
        ui.add_space(14.0);

        let full = ui.available_rect_before_wrap();
        let card_w = ((full.width() - 80.0) / 2.0).clamp(140.0, 300.0);
        let card_h = 150.0;
        ui.allocate_rect(
            Rect::from_min_size(full.min, vec2(full.width(), card_h + 10.0)),
            Sense::hover(),
        );

        if self.finished {
            let rect = Rect::from_min_size(full.min, vec2(card_w, card_h));
            song_card(ui, rect, &self.round[0], raised(), theme::accent());
            return None;
        }

        let a = &self.round[self.index * 2];
        let b = &self.round[self.index * 2 + 1];
        let left = Rect::from_min_size(full.min, vec2(card_w, card_h));
        let right = Rect::from_min_size(full.min + vec2(card_w + 80.0, 0.0), vec2(card_w, card_h));
        let mut chosen = None;
        for (rect, track, side) in [(left, a, 0), (right, b, 1)] {
            let response = ui.interact(rect, ui.scope_id().with(("duel", side)), Sense::click());
            let stroke = if response.hovered() {
                theme::accent()
            } else {
                theme::border()
            };
            let fill = if response.hovered() {
                theme::hover()
            } else {
                raised()
            };
            song_card(ui, rect, track, fill, stroke);
            if response.clicked() {
                chosen = Some(track.clone());
            }
        }
        let vs = pos2(left.right() + 40.0, left.center().y);
        ui.painter().circle_filled(vs, 24.0, theme::accent());
        ui.painter().text(
            vs,
            egui::Align2::CENTER_CENTER,
            "VS",
            theme::bold(16.0),
            theme::on_accent(),
        );

        let winner = chosen?;
        self.winners.push(winner);
        self.index += 1;
        if self.index * 2 >= self.round.len() {
            self.round = std::mem::take(&mut self.winners);
            self.index = 0;
            self.round_number += 1;
            if self.round.len() == 1 {
                self.finished = true;
                return Some((self.round.clone(), "The winner of This or That".to_string()));
            }
        }
        None
    }
}

// ---- Time Machine ----

#[derive(Default)]
pub struct TimeMachine;

impl TimeMachine {
    pub fn ui(&mut self, ui: &mut Ui, pool: &[Track]) -> Option<(Vec<Track>, String)> {
        let mut per_year: BTreeMap<u16, usize> = BTreeMap::new();
        for t in pool.iter().filter(|t| t.year > 0) {
            *per_year.entry(t.year).or_default() += 1;
        }
        let (Some((&first, _)), Some((&last, _))) =
            (per_year.first_key_value(), per_year.last_key_value())
        else {
            ui.label(RichText::new("These songs have no release years.").color(subdued()));
            return None;
        };
        let most = *per_year.values().max().unwrap_or(&1);

        ui.label(RichText::new("Click a year, or pick a whole decade below.").color(subdued()));
        ui.add_space(8.0);

        // Bar chart of songs per year.
        let full = ui.available_rect_before_wrap();
        let chart_h = 180.0;
        let years = (last - first + 1) as f32;
        let bar_w = (full.width() / years).clamp(3.0, 28.0);
        let chart = Rect::from_min_size(full.min, vec2(bar_w * years, chart_h));
        ui.allocate_rect(
            Rect::from_min_size(full.min, vec2(full.width(), chart_h + 26.0)),
            Sense::hover(),
        );
        let p = ui.painter();
        p.hline(
            chart.x_range(),
            chart.bottom(),
            Stroke::new(1.0, theme::border()),
        );
        let mut result = None;
        for year in first..=last {
            let x = chart.left() + (year - first) as f32 * bar_w;
            let column = Rect::from_min_max(pos2(x, chart.top()), pos2(x + bar_w, chart.bottom()));
            let count = per_year.get(&year).copied().unwrap_or(0);
            let response = ui.interact(column, ui.scope_id().with(("year", year)), Sense::click());
            let h = chart_h * count as f32 / most as f32;
            let bar = Rect::from_min_max(
                pos2(x + 1.0, chart.bottom() - h),
                pos2(x + bar_w - 1.0, chart.bottom()),
            );
            let color = if response.hovered() {
                theme::accent()
            } else {
                theme::accent().gamma_multiply(0.55)
            };
            if count > 0 {
                p.rect_filled(bar, CornerRadius::same(2), color);
            }
            if year % 10 == 0 || year == first {
                p.text(
                    pos2(x, chart.bottom() + 4.0),
                    egui::Align2::LEFT_TOP,
                    year.to_string(),
                    theme::regular(11.0),
                    muted(),
                );
            }
            if count > 0 {
                let response = response.on_hover_text(format!("{year}: {count} songs"));
                if response.clicked() {
                    result = Some((
                        shuffled(pool, 50, |t| t.year == year),
                        format!("Songs from {year}"),
                    ));
                }
            }
        }
        ui.add_space(8.0);

        // Decade buttons.
        let mut per_decade: BTreeMap<u16, usize> = BTreeMap::new();
        for (year, count) in &per_year {
            *per_decade.entry(year / 10 * 10).or_default() += count;
        }
        ui.horizontal_wrapped(|ui| {
            for (decade, count) in per_decade {
                let label = format!("{decade}s · {count}");
                if widgets::button(ui, &label, ButtonStyle::Ghost, 13.0).clicked() {
                    result = Some((
                        shuffled(pool, 50, |t| t.year / 10 * 10 == decade),
                        format!("A trip to the {decade}s"),
                    ));
                }
            }
        });
        result
    }
}

// ---- Rediscover ----

/// Songs from the oldest quarter of the list (by the date they were saved).
pub fn rediscover_ui(ui: &mut Ui, pool: &[Track]) -> Option<(Vec<Track>, String)> {
    let mut dated: Vec<&Track> = pool.iter().filter(|t| t.added_at.len() >= 10).collect();
    if dated.len() < 4 {
        ui.label(
            RichText::new("This only works with lists that remember when songs were added, like Liked Songs and playlists.")
                .color(subdued()),
        );
        return None;
    }
    dated.sort_by(|a, b| a.added_at.cmp(&b.added_at));
    let oldest: Vec<Track> = dated[..(dated.len() / 4).max(10).min(dated.len())]
        .iter()
        .map(|t| (*t).clone())
        .collect();
    let from = &oldest[0].added_at[..10];
    let to = &oldest[oldest.len() - 1].added_at[..10];

    let card = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(card.min, vec2(card.width().min(560.0), 170.0));
    ui.allocate_rect(rect, Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        raised(),
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
    let mut result = None;
    widgets::place(
        ui,
        egui::UiBuilder::new()
            .max_rect(rect.shrink(20.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.label(
                RichText::new(format!("{} songs you saved long ago", oldest.len()))
                    .font(theme::bold(20.0))
                    .color(theme::text()),
            );
            ui.label(RichText::new(format!("Added between {from} and {to}.")).color(subdued()));
            ui.label(
                RichText::new("Remember these? Here's a shuffled bunch of them.").color(subdued()),
            );
            ui.add_space(14.0);
            if widgets::button(ui, "Play 25 forgotten songs", ButtonStyle::Accent, 15.0).clicked() {
                result = Some((
                    shuffled(&oldest, 25, |_| true),
                    "Rediscovered songs".to_string(),
                ));
            }
        },
    );
    result
}

// ---- Surprise Mix ----

/// Returns true when the user asks for a mix.
pub fn surprise_mix_ui(ui: &mut Ui, loading: bool) -> bool {
    let card = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(card.min, vec2(card.width().min(560.0), 170.0));
    ui.allocate_rect(rect, Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        raised(),
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
    let mut clicked = false;
    widgets::place(
        ui,
        egui::UiBuilder::new()
            .max_rect(rect.shrink(20.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.label(
                RichText::new("30 songs from all over your library")
                    .font(theme::bold(20.0))
                    .color(theme::text()),
            );
            ui.label(RichText::new("Takes a handful of songs from Liked Songs and up to six random playlists, then shuffles them together.").color(subdued()));
            ui.add_space(14.0);
            if loading {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(18.0).color(theme::accent()));
                    ui.label(RichText::new("Mixing…").color(subdued()));
                });
            } else if widgets::button(ui, "Make me a mix", ButtonStyle::Accent, 15.0).clicked() {
                clicked = true;
            }
        },
    );
    clicked
}

// ---- Daily Pick ----

/// One song per day, the same all day long. Each song gets a score from the
/// date and its link; the highest score wins. Adding songs to the list rarely
/// changes today's pick.
#[derive(Default)]
pub struct Daily {
    /// The date the song was revealed for.
    revealed: Option<jiff::civil::Date>,
}

impl Daily {
    pub fn ui(&mut self, ui: &mut Ui, pool: &[Track]) -> Option<(Vec<Track>, String)> {
        let today = jiff::Zoned::now().date();
        let pick = daily_pick(pool, today)?;
        let revealed = self.revealed == Some(today);
        ui.label(
            RichText::new(today.strftime("%A %-d %B %Y").to_string())
                .font(theme::bold(16.0))
                .color(theme::text()),
        );
        ui.label(RichText::new("Come back tomorrow for a new one.").color(subdued()));
        ui.add_space(14.0);

        let full = ui.available_rect_before_wrap();
        let card = Rect::from_min_size(full.min, vec2(full.width().min(320.0), 150.0));
        let response = ui.allocate_rect(card, Sense::click());
        if revealed {
            song_card(ui, card, pick, raised(), theme::accent());
            return None;
        }
        let hovered = response.hovered();
        ui.painter().rect(
            card,
            CornerRadius::same(10),
            if hovered { theme::hover() } else { raised() },
            Stroke::new(1.5, theme::accent()),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            card.center() - vec2(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            "?",
            theme::bold(44.0),
            theme::accent(),
        );
        ui.painter().text(
            card.center() + vec2(0.0, 36.0),
            egui::Align2::CENTER_CENTER,
            "Click to see today's song",
            theme::regular(13.0),
            subdued(),
        );
        if response.clicked() {
            self.revealed = Some(today);
            return Some((vec![pick.clone()], "Today's pick".to_string()));
        }
        None
    }
}

fn daily_pick(pool: &[Track], date: jiff::civil::Date) -> Option<&Track> {
    use std::hash::{Hash, Hasher};
    pool.iter().max_by_key(|t| {
        // A fixed hash (not the randomised std one), so the pick is the same all day.
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        (date.year(), date.month(), date.day(), &t.uri).hash(&mut h);
        h.finish()
    })
}

/// The FNV-1a hash: tiny and the same on every run.
struct Fnv(u64);

impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        // Mix the bits a little more so nearby dates give unrelated picks.
        let mut x = self.0;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^ (x >> 33)
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}
