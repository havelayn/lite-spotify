//! The Fun tab: little games that pick music for you when you don't know what
//! you want to hear. Everything here is drawn with egui shapes; nothing is
//! downloaded, so it costs almost no memory. Animations only redraw the window
//! while they are moving.

mod claw;
mod pickers;
mod roulette;
mod slots;

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Ui, vec2};
use rand::seq::IndexedRandom;

use crate::api::{Playlist, Track, TrackSource};
use crate::theme::{self, MUTED, RAISED, SUBDUED, TEXT};
use crate::widgets::{self, ButtonStyle, Icon};

/// What the Fun tab asks the app to do.
pub enum FunAction {
    /// Start playing these songs (the first one now, the rest as the queue).
    Play(Vec<Track>),
    /// Add these songs after the current one.
    Queue(Vec<Track>),
    /// Load the songs of this list to play with.
    LoadPool(TrackSource),
    /// Build a "Surprise Mix" from random songs of random playlists.
    MakeSurpriseMix,
    /// Save these songs as a new playlist with this name.
    SaveAsPlaylist(String, Vec<Track>),
}

#[derive(Clone, Copy, PartialEq)]
enum Game {
    Roulette,
    Claw,
    Slots,
    Cards,
    Duel,
    TimeMachine,
    Rediscover,
    SurpriseMix,
}

impl Game {
    const ALL: [Game; 8] = [
        Game::Roulette,
        Game::Claw,
        Game::Slots,
        Game::Cards,
        Game::Duel,
        Game::TimeMachine,
        Game::Rediscover,
        Game::SurpriseMix,
    ];

    fn name(self) -> &'static str {
        match self {
            Game::Roulette => "Song Roulette",
            Game::Claw => "Claw Machine",
            Game::Slots => "Slot Machine",
            Game::Cards => "Mystery Cards",
            Game::Duel => "This or That",
            Game::TimeMachine => "Time Machine",
            Game::Rediscover => "Rediscover",
            Game::SurpriseMix => "Surprise Mix",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Game::Roulette => "Spin the wheel. Wherever the ball lands, that's your song.",
            Game::Claw => "Steer the claw, drop it and try to grab a prize. It slips sometimes!",
            Game::Slots => "Pull the lever for a line-up of three songs.",
            Game::Cards => "Six face-down cards. Pick one and see what you get.",
            Game::Duel => "Two songs at a time: click the one you like more until one is left.",
            Game::TimeMachine => "Pick a year and hear songs from back then.",
            Game::Rediscover => "Songs you saved a long time ago and may have forgotten.",
            Game::SurpriseMix => "A mix of random songs from all over your playlists.",
        }
    }

    /// Whether the game plays with the songs of the chosen list.
    fn uses_pool(self) -> bool {
        self != Game::SurpriseMix
    }
}

/// The songs a game picked, waiting for the user to play or queue them.
struct Pick {
    tracks: Vec<Track>,
    title: String,
}

pub struct Fun {
    game: Game,
    /// Where the songs come from, and the songs themselves (playable songs only).
    pool_source: TrackSource,
    pool: Vec<Track>,
    pool_id: u64,
    pool_loading: bool,
    pool_error: Option<String>,
    /// Bumped when the pool changes, so games deal new songs.
    pool_version: u64,
    pick: Option<Pick>,
    mix_loading: bool,

    roulette: roulette::Roulette,
    claw: claw::Claw,
    slots: slots::Slots,
    cards: pickers::Cards,
    duel: pickers::Duel,
    time_machine: pickers::TimeMachine,
}

impl Fun {
    pub fn new() -> Self {
        Self {
            game: Game::Roulette,
            pool_source: TrackSource::LikedSongs,
            pool: Vec::new(),
            pool_id: 0,
            pool_loading: false,
            pool_error: None,
            pool_version: 0,
            pick: None,
            mix_loading: false,
            roulette: roulette::Roulette::default(),
            claw: claw::Claw::default(),
            slots: slots::Slots::default(),
            cards: pickers::Cards::default(),
            duel: pickers::Duel::default(),
            time_machine: pickers::TimeMachine,
        }
    }

    /// Shows one game, for screenshots of the demo data (debug builds only).
    #[cfg(debug_assertions)]
    pub fn demo_select(&mut self, name: &str) {
        self.game = match name {
            "claw" => Game::Claw,
            "slots" => Game::Slots,
            "cards" => Game::Cards,
            "duel" => Game::Duel,
            "time" => Game::TimeMachine,
            "rediscover" => Game::Rediscover,
            "mix" => Game::SurpriseMix,
            _ => Game::Roulette,
        };
    }

    /// The id of the request loading the pool, so its pages can be matched up.
    pub fn pool_id(&self) -> u64 {
        self.pool_id
    }

    pub fn needs_pool(&self) -> bool {
        self.pool.is_empty() && !self.pool_loading && self.pool_error.is_none()
    }

    pub fn pool_source(&self) -> &TrackSource {
        &self.pool_source
    }

    /// Called by the app when it starts loading a new pool.
    pub fn start_pool(&mut self, source: TrackSource, id: u64) {
        self.pool_source = source;
        self.pool_id = id;
        self.pool.clear();
        self.pool_loading = true;
        self.pool_error = None;
        self.pool_version += 1;
    }

    pub fn add_to_pool(&mut self, tracks: Vec<Track>) {
        self.pool
            .extend(tracks.into_iter().filter(|t| t.playable && !t.is_episode));
        self.pool_version += 1;
    }

    pub fn pool_done(&mut self, error: Option<String>) {
        self.pool_loading = false;
        if self.pool.is_empty() && error.is_none() {
            self.pool_error = Some("There are no playable songs in this list.".into());
        } else {
            self.pool_error = error;
        }
    }

    pub fn mix_done(
        &mut self,
        result: Result<Vec<Track>, String>,
        autoplay: bool,
    ) -> Option<FunAction> {
        self.mix_loading = false;
        match result {
            Ok(tracks) if !tracks.is_empty() => {
                self.picked(tracks, "Your Surprise Mix".into(), autoplay)
            }
            Ok(_) => {
                self.pool_error = Some("Could not find songs for a mix.".into());
                None
            }
            Err(e) => {
                self.pool_error = Some(e);
                None
            }
        }
    }

    fn picked(&mut self, tracks: Vec<Track>, title: String, autoplay: bool) -> Option<FunAction> {
        if autoplay {
            self.pick = Some(Pick {
                tracks: tracks.clone(),
                title,
            });
            Some(FunAction::Play(tracks))
        } else {
            self.pick = Some(Pick { tracks, title });
            None
        }
    }

    /// Draws the Fun tab. Returns what the app should do.
    pub fn ui(&mut self, ui: &mut Ui, playlists: &[Playlist], autoplay: bool) -> Vec<FunAction> {
        let mut actions = Vec::new();

        // Left: the list of games.
        let full = ui.max_rect();
        let menu_w = 210.0;
        let menu = Rect::from_min_size(full.min, vec2(menu_w, full.height()));
        widgets::place(
            ui,
            egui::UiBuilder::new().max_rect(menu.shrink(12.0)),
            |ui| {
                ui.label(
                    RichText::new("Can't decide?")
                        .font(theme::bold(18.0))
                        .color(TEXT),
                );
                ui.label(RichText::new("Let a game pick for you.").color(SUBDUED));
                ui.add_space(10.0);
                for game in Game::ALL {
                    let selected = self.game == game;
                    if widgets::nav_row(ui, game_icon(game), game.name(), selected, false).clicked()
                    {
                        self.game = game;
                        self.pick = None;
                    }
                }
            },
        );
        ui.painter().vline(
            menu.right(),
            menu.y_range(),
            egui::Stroke::new(1.0, theme::BORDER),
        );

        // Right: the game.
        let area = Rect::from_min_max(
            egui::pos2(menu.right() + 20.0, full.top() + 16.0),
            full.max - vec2(20.0, 16.0),
        );
        widgets::place(
            ui,
            egui::UiBuilder::new()
                .max_rect(area)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.label(
                    RichText::new(self.game.name())
                        .font(theme::bold(24.0))
                        .color(TEXT),
                );
                ui.label(RichText::new(self.game.blurb()).color(SUBDUED));
                ui.add_space(8.0);
                if self.game.uses_pool() {
                    self.pool_picker(ui, playlists, &mut actions);
                    ui.add_space(8.0);
                }
                let pick_height = if self.pick.is_some() { 96.0 } else { 0.0 };
                let game_rect = Rect::from_min_max(
                    ui.cursor().min,
                    egui::pos2(area.right(), area.bottom() - pick_height),
                );
                let picked = widgets::place(
                    ui,
                    egui::UiBuilder::new()
                        .max_rect(game_rect)
                        .layout(Layout::top_down(Align::Min)),
                    |ui| self.game_ui(ui, &mut actions),
                );
                if let Some((tracks, title)) = picked
                    && let Some(action) = self.picked(tracks, title, autoplay)
                {
                    actions.push(action);
                }
                if self.pick.is_some() {
                    let rect = Rect::from_min_max(
                        egui::pos2(area.left(), area.bottom() - pick_height + 8.0),
                        area.max,
                    );
                    self.pick_card(ui, rect, &mut actions);
                }
            },
        );
        actions
    }

    fn pool_picker(&mut self, ui: &mut Ui, playlists: &[Playlist], actions: &mut Vec<FunAction>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Songs from").color(SUBDUED));
            let current = source_name(&self.pool_source);
            egui::ComboBox::from_id_salt("fun-pool")
                .selected_text(current)
                .width(260.0)
                .show_ui(ui, |ui| {
                    let liked = TrackSource::LikedSongs;
                    if ui
                        .selectable_label(self.pool_source == liked, "Liked Songs")
                        .clicked()
                    {
                        actions.push(FunAction::LoadPool(liked));
                    }
                    for p in playlists {
                        let source = TrackSource::Playlist {
                            id: p.id.clone(),
                            name: p.name.clone(),
                        };
                        if ui
                            .selectable_label(self.pool_source == source, p.name.as_str())
                            .clicked()
                        {
                            actions.push(FunAction::LoadPool(source));
                        }
                    }
                });
            if self.pool_loading {
                ui.add(egui::Spinner::new().size(14.0).color(SUBDUED));
                ui.label(RichText::new(format!("{} songs so far…", self.pool.len())).color(MUTED));
            } else if !self.pool.is_empty() {
                ui.label(RichText::new(format!("{} songs", self.pool.len())).color(MUTED));
            }
        });
        if let Some(e) = &self.pool_error {
            ui.label(RichText::new(e).color(theme::ERROR));
        }
    }

    /// Draws the selected game. Returns songs when the game picked some.
    fn game_ui(
        &mut self,
        ui: &mut Ui,
        actions: &mut Vec<FunAction>,
    ) -> Option<(Vec<Track>, String)> {
        if self.game == Game::SurpriseMix {
            if pickers::surprise_mix_ui(ui, self.mix_loading) {
                self.mix_loading = true;
                actions.push(FunAction::MakeSurpriseMix);
            }
            return None;
        }
        if self.pool.len() < 2 {
            if !self.pool_loading {
                ui.add_space(20.0);
                ui.label(
                    RichText::new("Pick a list with a few songs in it to play.").color(SUBDUED),
                );
            } else {
                ui.add_space(20.0);
                ui.add(egui::Spinner::new().size(28.0).color(theme::accent()));
            }
            return None;
        }
        let pool = &self.pool;
        let version = self.pool_version;
        match self.game {
            Game::Roulette => self.roulette.ui(ui, pool, version),
            Game::Claw => self.claw.ui(ui, pool, version),
            Game::Slots => self.slots.ui(ui, pool, version),
            Game::Cards => self.cards.ui(ui, pool, version),
            Game::Duel => self.duel.ui(ui, pool, version),
            Game::TimeMachine => self.time_machine.ui(ui, pool),
            Game::Rediscover => pickers::rediscover_ui(ui, pool),
            Game::SurpriseMix => None,
        }
    }

    /// The card at the bottom showing what a game picked.
    fn pick_card(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<FunAction>) {
        let Some(pick) = &self.pick else { return };
        ui.painter().rect(
            rect,
            CornerRadius::same(10),
            RAISED,
            egui::Stroke::new(1.5, theme::accent()),
            egui::StrokeKind::Inside,
        );
        let mut close = false;
        widgets::place_in(ui, rect.shrink2(vec2(16.0, 10.0)), |ui| {
            let (icon_rect, _) = ui.allocate_exact_size(egui::Vec2::splat(44.0), Sense::hover());
            ui.painter()
                .circle_filled(icon_rect.center(), 22.0, theme::accent_soft());
            widgets::paint_icon(
                ui.painter(),
                icon_rect.shrink(12.0),
                Icon::Note,
                theme::accent(),
            );
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 460.0).max(120.0));
                ui.label(RichText::new(&pick.title).small().color(theme::accent()));
                let first = &pick.tracks[0];
                let text = if pick.tracks.len() == 1 {
                    first.name.clone()
                } else {
                    format!("{} + {} more", first.name, pick.tracks.len() - 1)
                };
                ui.add(
                    egui::Label::new(RichText::new(text).font(theme::bold(16.0)).color(TEXT))
                        .truncate(),
                );
                ui.add(egui::Label::new(RichText::new(&first.artists).color(SUBDUED)).truncate());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::Close, 12.0, true, false)
                    .on_hover_text("Close")
                    .clicked()
                {
                    close = true;
                }
                if pick.tracks.len() > 1
                    && widgets::button(ui, "Save as playlist", ButtonStyle::Ghost, 13.0).clicked()
                {
                    actions.push(FunAction::SaveAsPlaylist(
                        pick.title.clone(),
                        pick.tracks.clone(),
                    ));
                }
                if widgets::button(ui, "Add to queue", ButtonStyle::Ghost, 13.0).clicked() {
                    actions.push(FunAction::Queue(pick.tracks.clone()));
                    close = true;
                }
                if widgets::button(ui, "Play now", ButtonStyle::Accent, 13.0).clicked() {
                    actions.push(FunAction::Play(pick.tracks.clone()));
                }
            });
        });
        if close {
            self.pick = None;
        }
    }
}

fn game_icon(game: Game) -> Icon {
    match game {
        Game::Roulette => Icon::Disc,
        Game::Claw => Icon::Plus,
        Game::Slots => Icon::Repeat,
        Game::Cards => Icon::Heart,
        Game::Duel => Icon::Shuffle,
        Game::TimeMachine => Icon::Previous,
        Game::Rediscover => Icon::Search,
        Game::SurpriseMix => Icon::Note,
    }
}

pub fn source_name(source: &TrackSource) -> String {
    match source {
        TrackSource::LikedSongs => "Liked Songs".into(),
        TrackSource::Playlist { name, .. }
        | TrackSource::Album { name, .. }
        | TrackSource::Show { name, .. } => name.clone(),
    }
}

/// `n` different random songs from `pool`.
fn sample(pool: &[Track], n: usize) -> Vec<Track> {
    pool.choose_multiple(&mut rand::rng(), n).cloned().collect()
}

/// Colours for capsules, cards and wheel labels.
const PARTY: [Color32; 6] = [
    Color32::from_rgb(0xff, 0x6b, 0x6b),
    Color32::from_rgb(0xff, 0xb0, 0x2e),
    Color32::from_rgb(0x2e, 0xd3, 0xb7),
    Color32::from_rgb(0x5c, 0xb4, 0xff),
    Color32::from_rgb(0xa5, 0x8b, 0xff),
    Color32::from_rgb(0xff, 0x8f, 0xd1),
];

/// A small card with a song's title and artist.
fn song_card(ui: &Ui, rect: Rect, track: &Track, fill: Color32, stroke: Color32) {
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(8),
        fill,
        egui::Stroke::new(1.5, stroke),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink2(vec2(10.0, 8.0));
    let title = p.layout(track.name.clone(), theme::bold(14.0), TEXT, inner.width());
    let title_h = title.size().y.min(inner.height() * 0.66);
    let clip = p.with_clip_rect(inner);
    clip.galley(inner.left_top(), title, TEXT);
    clip.text(
        egui::pos2(inner.left(), inner.top() + title_h + 4.0),
        egui::Align2::LEFT_TOP,
        &track.artists,
        theme::regular(12.0),
        SUBDUED,
    );
}

/// Smooth slow-down curve for spins: fast at first, gently stopping at the end.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}
