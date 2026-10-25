//! The app's state and what happens behind the screens: signing in, loading
//! lists, the play queue and talking to the player. The screens are in `app/ui.rs`.

mod ui;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, TextureHandle};
use rand::seq::{IndexedRandom, SliceRandom};

use crate::api::{self, Playlist, SearchResults, Show, Track, TrackSource, WebApi};
use crate::auth;
use crate::config::{self, Config, Repeat};
use crate::fun::{Fun, FunAction};
use crate::player::{PlayerCtl, PlayerMsg};
use crate::theme;

/// How long a message at the bottom of the window stays visible.
const NOTICE_TIME: Duration = Duration::from_secs(6);

/// Messages from background tasks to the UI thread.
enum Msg {
    SignInUrl(String),
    SignedIn(Arc<WebApi>, Option<String>),
    SignInFailed(String),
    NewRefreshToken(String),
    UserName(String),
    Playlists(Result<Vec<Playlist>, String>),
    Shows(Result<Vec<Show>, String>),
    TracksPage {
        id: u64,
        tracks: Vec<Track>,
    },
    TracksDone {
        id: u64,
        error: Option<String>,
    },
    Search {
        id: u64,
        result: Result<SearchResults, String>,
    },
    Mix(Result<Vec<Track>, String>),
    Cover {
        url: String,
        image: egui::ColorImage,
    },
    Player(PlayerMsg),
}

/// Sends messages to the UI and wakes it up.
#[derive(Clone)]
struct Outbox {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Outbox {
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

enum Screen {
    /// First run: ask for the developer app's Client ID.
    Setup {
        input: String,
        error: Option<String>,
    },
    /// Waiting for the user to sign in (or for a saved login to be checked).
    SignIn {
        url: Option<String>,
        busy: bool,
        error: Option<String>,
    },
    Main,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Library,
    Search,
    Podcasts,
    Fun,
    Settings,
}

/// A list of songs or episodes being shown.
#[derive(Default)]
struct ListView {
    id: u64,
    source: Option<TrackSource>,
    tracks: Vec<Track>,
    loading: bool,
    error: Option<String>,
}

#[derive(Default)]
struct SearchView {
    id: u64,
    query: String,
    results: Option<SearchResults>,
    loading: bool,
    error: Option<String>,
}

enum PlayerState {
    Off,
    Connecting,
    BrowserLogin(String),
    Ready,
    Failed(String),
}

#[derive(Default)]
struct NowPlaying {
    track: Option<Track>,
    playing: bool,
    loading: bool,
    /// Position at `since`; while playing the real position keeps moving from there.
    position_ms: u32,
    since: Option<Instant>,
}

impl NowPlaying {
    fn position_ms(&self) -> u32 {
        let extra = self
            .since
            .map(|t| t.elapsed().as_millis() as u32)
            .unwrap_or(0);
        let pos = self.position_ms.saturating_add(extra);
        match &self.track {
            Some(t) if t.duration_ms > 0 => pos.min(t.duration_ms),
            _ => pos,
        }
    }
}

pub struct App {
    rt: tokio::runtime::Runtime,
    http: reqwest::Client,
    rx: Receiver<Msg>,
    outbox: Outbox,
    config: Config,
    screen: Screen,
    tab: Tab,

    api: Option<Arc<WebApi>>,
    user_name: String,
    playlists: Vec<Playlist>,
    playlists_loading: bool,
    shows: Vec<Show>,
    shows_loading: bool,
    shows_loaded: bool,
    library: ListView,
    podcast: ListView,
    search: SearchView,
    search_input: String,
    next_id: u64,
    /// A short message shown at the bottom of the window, and when it appeared.
    notice: Option<(String, Instant)>,

    player: Option<PlayerCtl>,
    player_state: PlayerState,
    queue: Vec<Track>,
    queue_pos: usize,
    /// How many songs right after the current one were added with "Add to queue".
    user_queued: usize,
    /// The queue in its original order while shuffle is on, to go back to.
    unshuffled: Option<Vec<Track>>,
    /// The list the queue was started from, to highlight it.
    playing_source: Option<TrackSource>,
    now: NowPlaying,
    /// Seek bar position (0..1) while the user is dragging it.
    seek_drag: Option<f32>,
    skipped_in_a_row: usize,
    cover: Option<(String, TextureHandle)>,
    cover_requested: Option<String>,
    volume_before_mute: f32,

    /// The browser sign-in that is waiting for the user, so it can be cancelled.
    sign_in_task: Option<tokio::task::JoinHandle<()>>,
    /// When the redirect URI was copied, to show "Copied" for a moment.
    copied_at: Option<Instant>,
    /// The Fun tab's games. `None` while the Fun tab is turned off, so it takes no memory.
    fun: Option<Box<Fun>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Runtime) -> Self {
        let (tx, rx) = channel();
        let outbox = Outbox {
            tx,
            ctx: cc.egui_ctx.clone(),
        };
        let http = reqwest::Client::builder()
            .user_agent(concat!("lite-spotify/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("could not create the HTTP client");
        let config = Config::load();
        theme::setup(&cc.egui_ctx, &config.accent);

        let mut app = Self {
            rt,
            http,
            rx,
            outbox,
            screen: Screen::Setup {
                input: String::new(),
                error: None,
            },
            tab: Tab::Library,
            fun: config.fun_enabled.then(|| Box::new(Fun::new())),
            volume_before_mute: 0.5,
            config,
            api: None,
            user_name: String::new(),
            playlists: Vec::new(),
            playlists_loading: false,
            shows: Vec::new(),
            shows_loading: false,
            shows_loaded: false,
            library: ListView::default(),
            podcast: ListView::default(),
            search: SearchView::default(),
            search_input: String::new(),
            next_id: 1,
            notice: None,
            player: None,
            player_state: PlayerState::Off,
            queue: Vec::new(),
            queue_pos: 0,
            user_queued: 0,
            unshuffled: None,
            playing_source: None,
            now: NowPlaying::default(),
            seek_drag: None,
            skipped_in_a_row: 0,
            cover: None,
            cover_requested: None,
            sign_in_task: None,
            copied_at: None,
        };

        #[cfg(debug_assertions)]
        if let Ok(which) = std::env::var("LITE_SPOTIFY_DEMO") {
            app.fill_demo_data(&which);
            return app;
        }

        if app.config.client_id.is_empty() {
            app.screen = Screen::Setup {
                input: String::new(),
                error: None,
            };
        } else if let Some(refresh_token) = app.config.refresh_token.clone() {
            app.resume_sign_in(refresh_token);
        } else {
            app.screen = Screen::SignIn {
                url: None,
                busy: false,
                error: None,
            };
        }
        app
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
    }

    // ---------------------------------------------------------------- sign in

    fn on_new_refresh_token(&self) -> impl Fn(String) + Send + Sync + 'static {
        let outbox = self.outbox.clone();
        move |token| outbox.send(Msg::NewRefreshToken(token))
    }

    /// Signs in with the saved refresh token, without the browser.
    fn resume_sign_in(&mut self, refresh_token: String) {
        self.screen = Screen::SignIn {
            url: None,
            busy: true,
            error: None,
        };
        let (http, client_id, outbox) = (
            self.http.clone(),
            self.config.client_id.clone(),
            self.outbox.clone(),
        );
        let on_new = self.on_new_refresh_token();
        self.rt.spawn(async move {
            match auth::refresh(&http, &client_id, &refresh_token).await {
                Ok(tokens) => {
                    let rt = tokens.refresh_token.clone();
                    let api = WebApi::new(http, client_id, tokens, on_new);
                    outbox.send(Msg::SignedIn(Arc::new(api), rt));
                }
                Err(e) => {
                    log::warn!("Saved login did not work: {e:#}");
                    outbox.send(Msg::SignInFailed(
                        "Your saved login has expired. Please sign in again.".into(),
                    ));
                }
            }
        });
    }

    fn start_browser_sign_in(&mut self) {
        self.screen = Screen::SignIn {
            url: None,
            busy: true,
            error: None,
        };
        let (http, client_id, outbox) = (
            self.http.clone(),
            self.config.client_id.clone(),
            self.outbox.clone(),
        );
        let on_new = self.on_new_refresh_token();
        self.sign_in_task = Some(self.rt.spawn(async move {
            let url_outbox = outbox.clone();
            let result = auth::login(
                &http,
                &client_id,
                auth::WEB_REDIRECT_URI,
                auth::WEB_SCOPES,
                |url| url_outbox.send(Msg::SignInUrl(url)),
            )
            .await;
            match result {
                Ok(tokens) => {
                    let rt = tokens.refresh_token.clone();
                    let api = WebApi::new(http, client_id, tokens, on_new);
                    outbox.send(Msg::SignedIn(Arc::new(api), rt));
                }
                Err(e) => outbox.send(Msg::SignInFailed(format!("{e:#}"))),
            }
        }));
    }

    fn cancel_sign_in(&mut self) {
        // Stops the local login server, which frees the port again.
        if let Some(task) = self.sign_in_task.take() {
            task.abort();
        }
        self.screen = Screen::SignIn {
            url: None,
            busy: false,
            error: None,
        };
    }

    fn on_signed_in(&mut self, api: Arc<WebApi>, refresh_token: Option<String>) {
        if refresh_token.is_some() {
            self.config.refresh_token = refresh_token;
            self.config.save();
        }
        self.api = Some(api.clone());
        self.screen = Screen::Main;

        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            if let Ok(name) = api.user_name().await {
                outbox.send(Msg::UserName(name));
            }
        });
        self.reload_playlists();
        self.open_list(TrackSource::LikedSongs);
        self.start_player();
    }

    fn start_player(&mut self) {
        let outbox = self.outbox.clone();
        self.player_state = PlayerState::Connecting;
        self.player = Some(PlayerCtl::start(
            self.rt.handle(),
            self.http.clone(),
            self.config.volume,
            move |msg| outbox.send(Msg::Player(msg)),
        ));
    }

    fn sign_out(&mut self) {
        if let Some(p) = &self.player {
            p.stop();
        }
        self.player = None;
        self.player_state = PlayerState::Off;
        self.api = None;
        self.config.refresh_token = None;
        self.config.save();
        // Forget the player's saved login too.
        let _ = std::fs::remove_dir_all(config::player_credentials_dir());
        self.playlists.clear();
        self.shows.clear();
        self.shows_loaded = false;
        self.library = ListView::default();
        self.podcast = ListView::default();
        self.search = SearchView::default();
        self.queue.clear();
        self.unshuffled = None;
        self.playing_source = None;
        self.now = NowPlaying::default();
        self.cover = None;
        self.user_name.clear();
        if self.fun.is_some() {
            self.fun = Some(Box::new(Fun::new()));
        }
        self.tab = Tab::Library;
        self.screen = Screen::SignIn {
            url: None,
            busy: false,
            error: None,
        };
    }

    // ------------------------------------------------------------- browsing

    fn reload_playlists(&mut self) {
        let Some(api) = self.api.clone() else { return };
        self.playlists_loading = true;
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = api.playlists().await.map_err(|e| format!("{e:#}"));
            outbox.send(Msg::Playlists(result));
        });
    }

    fn load_shows(&mut self) {
        let Some(api) = self.api.clone() else { return };
        self.shows_loading = true;
        self.shows_loaded = true;
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = api.saved_shows().await.map_err(|e| format!("{e:#}"));
            outbox.send(Msg::Shows(result));
        });
    }

    /// Starts loading the songs of `source`; pages arrive as `Msg::TracksPage` with `id`.
    fn load_tracks(&mut self, source: TrackSource, id: u64) {
        let Some(api) = self.api.clone() else { return };
        let outbox = self.outbox.clone();
        let player = self.player.clone();
        self.rt.spawn(async move {
            let mut got_any = false;
            let result = api
                .tracks(&source, |tracks| {
                    got_any = true;
                    outbox.send(Msg::TracksPage { id, tracks });
                })
                .await;
            let error = match result {
                Ok(()) => None,
                Err(e) => match (&source, player, is_forbidden(&e) && !got_any) {
                    // Spotify does not let developer apps read playlists made by
                    // others (e.g. Spotify's own mixes). The player's connection can.
                    (TrackSource::Playlist { id: pid, .. }, Some(player), true) => {
                        match player.playlist_tracks(pid).await {
                            Ok(tracks) => {
                                outbox.send(Msg::TracksPage { id, tracks });
                                None
                            }
                            Err(e2) => Some(format!(
                                "Spotify does not allow this app to read this playlist ({e}). \
                                 Loading it through the player failed too: {e2:#}"
                            )),
                        }
                    }
                    _ => Some(format!("{e:#}")),
                },
            };
            outbox.send(Msg::TracksDone { id, error });
        });
    }

    /// Shows a playlist, album or Liked Songs on the Library tab (or a podcast on the Podcasts tab).
    fn open_list(&mut self, source: TrackSource) {
        let id = self.new_id();
        let view = ListView {
            id,
            source: Some(source.clone()),
            tracks: Vec::new(),
            loading: true,
            error: None,
        };
        if matches!(source, TrackSource::Show { .. }) {
            self.podcast = view;
            self.tab = Tab::Podcasts;
        } else {
            self.library = view;
            self.tab = Tab::Library;
        }
        self.load_tracks(source, id);
    }

    fn start_search(&mut self) {
        let query = self.search_input.trim().to_string();
        let Some(api) = self.api.clone() else { return };
        if query.is_empty() {
            return;
        }
        let id = self.new_id();
        self.search = SearchView {
            id,
            query: query.clone(),
            results: None,
            loading: true,
            error: None,
        };
        self.tab = Tab::Search;
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = api.search(&query).await.map_err(|e| format!("{e:#}"));
            outbox.send(Msg::Search { id, result });
        });
    }

    // ------------------------------------------------------------------ fun

    fn set_fun_enabled(&mut self, enabled: bool) {
        self.config.fun_enabled = enabled;
        self.config.save();
        if enabled {
            if self.fun.is_none() {
                self.fun = Some(Box::new(Fun::new()));
            }
        } else {
            self.fun = None;
            if self.tab == Tab::Fun {
                self.tab = Tab::Library;
            }
        }
    }

    fn load_fun_pool(&mut self, source: TrackSource) {
        let id = self.new_id();
        if let Some(fun) = &mut self.fun {
            fun.start_pool(source.clone(), id);
        }
        self.load_tracks(source, id);
    }

    fn on_fun_action(&mut self, action: FunAction) {
        match action {
            FunAction::Play(tracks) => self.play_list(tracks, 0, None),
            FunAction::Queue(tracks) => self.add_to_queue(tracks),
            FunAction::LoadPool(source) => self.load_fun_pool(source),
            FunAction::MakeSurpriseMix => self.make_surprise_mix(),
        }
    }

    /// 30 random songs from Liked Songs and up to six random playlists.
    fn make_surprise_mix(&mut self) {
        let Some(api) = self.api.clone() else { return };
        let mut sources = vec![TrackSource::LikedSongs];
        sources.extend(
            self.playlists
                .choose_multiple(&mut rand::rng(), 6)
                .map(|p| TrackSource::Playlist {
                    id: p.id.clone(),
                    name: p.name.clone(),
                }),
        );
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let mut mix = Vec::new();
            for source in sources {
                // Lists Spotify won't let us read are simply skipped.
                if let Ok(tracks) = api.sample_tracks(&source, 150).await {
                    let playable: Vec<Track> = tracks
                        .into_iter()
                        .filter(|t| t.playable && !t.is_episode)
                        .collect();
                    mix.extend(playable.choose_multiple(&mut rand::rng(), 8).cloned());
                }
            }
            mix.sort_by(|a, b| a.uri.cmp(&b.uri));
            mix.dedup_by(|a, b| a.uri == b.uri);
            mix.shuffle(&mut rand::rng());
            mix.truncate(30);
            outbox.send(Msg::Mix(Ok(mix)));
        });
    }

    // ------------------------------------------------------------- playback

    fn player_ready(&self) -> bool {
        matches!(self.player_state, PlayerState::Ready)
    }

    /// Plays `tracks[index]` and makes `tracks` the queue for next/previous.
    fn play_list(&mut self, mut tracks: Vec<Track>, index: usize, source: Option<TrackSource>) {
        if tracks.is_empty() {
            return;
        }
        self.playing_source = source;
        self.user_queued = 0;
        self.skipped_in_a_row = 0;
        let mut index = index.min(tracks.len() - 1);
        if self.config.shuffle {
            self.unshuffled = Some(tracks.clone());
            // The chosen song first, the rest in random order.
            let first = tracks.remove(index);
            tracks.shuffle(&mut rand::rng());
            tracks.insert(0, first);
            index = 0;
        } else {
            self.unshuffled = None;
        }
        self.queue = tracks;
        self.play_at(index, true);
    }

    fn set_shuffle(&mut self, on: bool) {
        self.config.shuffle = on;
        self.config.save();
        let current = self.now.track.as_ref().map(|t| t.uri.clone());
        if on {
            if self.queue.len() > self.queue_pos + 1 {
                self.unshuffled = Some(self.queue.clone());
                self.queue[self.queue_pos + 1..].shuffle(&mut rand::rng());
            }
        } else if let Some(original) = self.unshuffled.take() {
            self.queue_pos = current
                .and_then(|uri| original.iter().position(|t| t.uri == uri))
                .unwrap_or(0);
            self.queue = original;
        }
        self.user_queued = 0;
    }

    fn cycle_repeat(&mut self) {
        self.config.repeat = match self.config.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
        self.config.save();
    }

    /// Adds songs right after the current one (after earlier added songs).
    fn add_to_queue(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        if self.now.track.is_none() || self.queue.is_empty() {
            self.play_list(tracks, 0, None);
            return;
        }
        let n = tracks.len();
        let at = (self.queue_pos + 1 + self.user_queued).min(self.queue.len());
        self.queue.splice(at..at, tracks);
        self.user_queued += n;
        let text = if n == 1 {
            "Added 1 song to the queue".to_string()
        } else {
            format!("Added {n} songs to the queue")
        };
        self.notify(text);
    }

    /// Plays the first playable track at or after (or before, if `forward` is
    /// false) `index` in the queue.
    fn play_at(&mut self, index: usize, forward: bool) -> bool {
        let Some(player) = &self.player else {
            return false;
        };
        if !self.player_ready() {
            self.notify("The player is not connected yet. Please wait a moment.");
            return false;
        }
        let mut i = index;
        loop {
            match self.queue.get(i) {
                Some(t) if t.playable => break,
                Some(_) if forward => i += 1,
                Some(_) if i > 0 => i -= 1,
                _ => return false,
            }
        }
        let track = self.queue[i].clone();
        player.load(&track.uri, 0);
        self.queue_pos = i;
        self.now = NowPlaying {
            track: Some(track),
            playing: false,
            loading: true,
            position_ms: 0,
            since: None,
        };
        self.request_cover();
        true
    }

    /// Jumps to a song further down the queue (from the Up next list).
    fn jump_to(&mut self, index: usize) {
        self.user_queued = self
            .user_queued
            .saturating_sub(index.saturating_sub(self.queue_pos));
        self.play_at(index, true);
    }

    fn next(&mut self) {
        self.user_queued = self.user_queued.saturating_sub(1);
        if self.play_at(self.queue_pos + 1, true) {
            return;
        }
        if self.config.repeat == Repeat::All && self.play_at(0, true) {
            return;
        }
        // End of the list.
        if let Some(p) = &self.player {
            p.stop();
        }
        self.now.playing = false;
        self.now.loading = false;
        self.now.position_ms = 0;
        self.now.since = None;
    }

    fn previous(&mut self) {
        if self.now.position_ms() > 3000 || self.queue_pos == 0 {
            self.seek(0);
        } else {
            self.play_at(self.queue_pos - 1, false);
        }
    }

    fn toggle_play(&mut self) {
        let Some(player) = &self.player else { return };
        if self.now.track.is_none() {
            return;
        }
        if self.now.playing {
            player.pause();
        } else {
            player.play();
        }
    }

    fn seek(&mut self, position_ms: u32) {
        if let Some(p) = &self.player {
            p.seek(position_ms);
            self.now.position_ms = position_ms;
            if self.now.since.is_some() {
                self.now.since = Some(Instant::now());
            }
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.config.volume = volume;
        if let Some(p) = &self.player {
            p.set_volume(volume);
        }
    }

    /// The song that will play after the current one, if any.
    fn upcoming(&self) -> Option<&Track> {
        let after = self
            .queue
            .iter()
            .skip(self.queue_pos + 1)
            .find(|t| t.playable);
        match (after, self.config.repeat) {
            (_, Repeat::One) => self.now.track.as_ref(),
            (None, Repeat::All) => self.queue.iter().find(|t| t.playable),
            (after, _) => after,
        }
    }

    fn request_cover(&mut self) {
        let Some(url) = self.now.track.as_ref().and_then(|t| t.image_url.clone()) else {
            self.cover = None;
            return;
        };
        if self.cover.as_ref().is_some_and(|(u, _)| *u == url)
            || self.cover_requested.as_ref() == Some(&url)
        {
            return;
        }
        let Some(api) = self.api.clone() else { return };
        self.cover_requested = Some(url.clone());
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let Ok(bytes) = api.image(&url).await else {
                return;
            };
            let Ok(img) = image::load_from_memory(&bytes) else {
                return;
            };
            // At most 300×300 pixels: sharp in the now-playing panel, under 400 KB of memory.
            let img = img.thumbnail(300, 300).to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            outbox.send(Msg::Cover { url, image });
        });
    }

    fn on_player_msg(&mut self, msg: PlayerMsg) {
        let current = self.now.track.as_ref().map(|t| t.uri.clone());
        let is_current = |uri: &str| current.as_deref() == Some(uri);
        match msg {
            PlayerMsg::Connecting => self.player_state = PlayerState::Connecting,
            PlayerMsg::NeedsBrowserLogin(url) => self.player_state = PlayerState::BrowserLogin(url),
            PlayerMsg::Ready => self.player_state = PlayerState::Ready,
            PlayerMsg::Failed(e) => {
                self.player_state = PlayerState::Failed(e);
                self.now.loading = false;
            }
            PlayerMsg::Loading { uri } if is_current(&uri) => self.now.loading = true,
            PlayerMsg::Playing { uri, position_ms } if is_current(&uri) => {
                self.skipped_in_a_row = 0;
                self.now.playing = true;
                self.now.loading = false;
                self.now.position_ms = position_ms;
                self.now.since = Some(Instant::now());
            }
            PlayerMsg::Paused { uri, position_ms } if is_current(&uri) => {
                self.now.playing = false;
                self.now.loading = false;
                self.now.position_ms = position_ms;
                self.now.since = None;
            }
            PlayerMsg::Stopped => {}
            PlayerMsg::PreloadNext => {
                if let (Some(p), Some(t)) = (&self.player, self.upcoming()) {
                    p.preload(&t.uri);
                }
            }
            PlayerMsg::EndOfTrack { uri } if is_current(&uri) => {
                if self.config.repeat == Repeat::One {
                    self.play_at(self.queue_pos, true);
                } else {
                    self.next();
                }
            }
            PlayerMsg::Unavailable { uri } if is_current(&uri) => {
                self.skipped_in_a_row += 1;
                if self.skipped_in_a_row > 5 {
                    self.notify("Several songs in a row could not be played. Playback stopped.");
                    self.now.loading = false;
                } else {
                    let name = self
                        .now
                        .track
                        .as_ref()
                        .map(|t| t.name.clone())
                        .unwrap_or_default();
                    self.notify(format!("\"{name}\" is not available, skipped it."));
                    self.next();
                }
            }
            _ => {} // an event about a track that is no longer current
        }
    }

    fn handle_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::SignInUrl(url) => {
                    if let Screen::SignIn { url: u, .. } = &mut self.screen {
                        *u = Some(url);
                    }
                }
                Msg::SignedIn(api, rt) => self.on_signed_in(api, rt),
                Msg::SignInFailed(e) => {
                    self.screen = Screen::SignIn {
                        url: None,
                        busy: false,
                        error: Some(e),
                    };
                }
                Msg::NewRefreshToken(token) => {
                    self.config.refresh_token = Some(token);
                    self.config.save();
                }
                Msg::UserName(name) => self.user_name = name,
                Msg::Playlists(result) => {
                    self.playlists_loading = false;
                    match result {
                        Ok(p) => self.playlists = p,
                        Err(e) => self.notify(format!("Could not load playlists: {e}")),
                    }
                }
                Msg::Shows(result) => {
                    self.shows_loading = false;
                    match result {
                        Ok(s) => self.shows = s,
                        Err(e) => self.notify(format!("Could not load podcasts: {e}")),
                    }
                }
                Msg::TracksPage { id, tracks } => {
                    if id == self.library.id {
                        self.library.tracks.extend(tracks);
                    } else if id == self.podcast.id {
                        self.podcast.tracks.extend(tracks);
                    } else if let Some(fun) = &mut self.fun
                        && fun.pool_id() == id
                    {
                        fun.add_to_pool(tracks);
                    }
                }
                Msg::TracksDone { id, error } => {
                    for view in [&mut self.library, &mut self.podcast] {
                        if view.id == id {
                            view.loading = false;
                            view.error = error.clone();
                        }
                    }
                    if let Some(fun) = &mut self.fun
                        && fun.pool_id() == id
                    {
                        fun.pool_done(error);
                    }
                }
                Msg::Search { id, result } => {
                    if id == self.search.id {
                        self.search.loading = false;
                        match result {
                            Ok(r) => self.search.results = Some(r),
                            Err(e) => self.search.error = Some(e),
                        }
                    }
                }
                Msg::Mix(result) => {
                    let autoplay = self.config.fun_autoplay;
                    let action = self.fun.as_mut().and_then(|f| f.mix_done(result, autoplay));
                    if let Some(action) = action {
                        self.on_fun_action(action);
                    }
                }
                Msg::Cover { url, image } => {
                    let wanted = self.now.track.as_ref().and_then(|t| t.image_url.as_ref());
                    if wanted == Some(&url) {
                        let texture =
                            ctx.load_texture("cover", image, egui::TextureOptions::LINEAR);
                        self.cover = Some((url, texture));
                    }
                    self.cover_requested = None;
                }
                Msg::Player(msg) => self.on_player_msg(msg),
            }
        }
    }

    /// Fake library for trying out the layout without a Spotify account (debug
    /// builds only). Set LITE_SPOTIFY_DEMO to library, search, podcasts,
    /// settings or fun (fun:claw, fun:slots, ... for a specific game).
    #[cfg(debug_assertions)]
    fn fill_demo_data(&mut self, which: &str) {
        self.screen = Screen::Main;
        self.user_name = "Demo".into();
        self.player_state = PlayerState::Ready;
        self.playlists = (1..=24)
            .map(|i| Playlist {
                id: format!("demo{i}"),
                name: format!("Playlist number {i} with a fairly long name"),
                owner: "Demo".into(),
                total: 20,
            })
            .collect();
        let artists = [
            "Artist One",
            "The Twos",
            "Band Three",
            "Four Seasons",
            "Fifth Ave",
        ];
        let tracks: Vec<Track> = (0..120u32)
            .map(|i| Track {
                uri: format!("spotify:track:demo{i}"),
                name: format!("Song title {i} \u{2014} 東京 Ünïcödé (Remastered 2011)"),
                artists: artists[i as usize % artists.len()].into(),
                album: "Some Album Name That Is Quite Long".into(),
                duration_ms: 200_000 + i * 1000,
                image_url: None,
                playable: i % 9 != 4,
                is_episode: false,
                year: 1965 + (i * 7 % 60) as u16,
                added_at: format!("20{:02}-0{}-15T10:00:00Z", 12 + i % 13, 1 + i % 9),
            })
            .collect();
        let source = TrackSource::Playlist {
            id: "demo2".into(),
            name: "Playlist number 2 with a fairly long name".into(),
        };
        self.library = ListView {
            id: 1,
            source: Some(source.clone()),
            tracks: tracks.clone(),
            loading: false,
            error: None,
        };
        self.playing_source = Some(source);
        self.queue = tracks.clone();
        self.queue_pos = 2;
        self.now = NowPlaying {
            track: Some(tracks[2].clone()),
            playing: true,
            loading: false,
            position_ms: 65_000,
            since: Some(Instant::now()),
        };
        self.shows = (1..=6)
            .map(|i| Show {
                id: format!("show{i}"),
                name: format!("Podcast number {i}"),
                publisher: "Someone".into(),
            })
            .collect();
        self.shows_loaded = true;
        match which {
            "search" => {
                self.tab = Tab::Search;
                self.search = SearchView {
                    id: 5,
                    query: "demo search".into(),
                    results: Some(SearchResults {
                        tracks: tracks[..6].to_vec(),
                        albums: (1..=5)
                            .map(|i| api::Album {
                                id: format!("album{i}"),
                                name: format!("Album {i}"),
                                artists: "Some Artist".into(),
                            })
                            .collect(),
                        playlists: self.playlists[..5].to_vec(),
                        shows: self.shows[..3].to_vec(),
                    }),
                    loading: false,
                    error: None,
                };
            }
            "podcasts" => {
                self.tab = Tab::Podcasts;
                self.podcast = ListView {
                    id: 6,
                    source: Some(TrackSource::Show {
                        id: "show1".into(),
                        name: "Podcast number 1".into(),
                    }),
                    tracks: (1..=20u32)
                        .map(|i| Track {
                            uri: format!("spotify:episode:demo{i}"),
                            name: format!("Episode {i}: a long talk about things"),
                            artists: "Podcast number 1".into(),
                            album: format!("2026-0{}-1{}", 1 + i % 9, i % 10),
                            duration_ms: 2_400_000 + i * 60_000,
                            image_url: None,
                            playable: true,
                            is_episode: true,
                            year: 2026,
                            added_at: String::new(),
                        })
                        .collect(),
                    loading: false,
                    error: None,
                };
            }
            "settings" => self.tab = Tab::Settings,
            game if game.starts_with("fun") => {
                self.tab = Tab::Fun;
                if let Some(fun) = &mut self.fun {
                    fun.start_pool(TrackSource::LikedSongs, 99);
                    fun.add_to_pool(tracks);
                    fun.pool_done(None);
                    fun.demo_select(game.trim_start_matches("fun").trim_start_matches(':'));
                }
            }
            _ => {}
        }
        self.notify("\"Song title 4\" is not available, skipped it.");
    }
}

fn is_forbidden(e: &anyhow::Error) -> bool {
    e.downcast_ref::<api::ApiError>().is_some_and(|e| {
        e.status == reqwest::StatusCode::FORBIDDEN || e.status == reqwest::StatusCode::NOT_FOUND
    })
}
