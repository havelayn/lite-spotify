//! The window: sign-in screens, sidebar, track lists and the now-playing bar.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, Key, Layout, RichText, Sense, TextureHandle, Vec2,
};

use crate::api::{self, Album, Playlist, SearchResults, Track, TrackSource, WebApi};
use crate::auth;
use crate::config::{self, Config};
use crate::player::{PlayerCtl, PlayerMsg};

const GREEN: Color32 = Color32::from_rgb(0x1d, 0xb9, 0x54);
const DIM: Color32 = Color32::from_rgb(0xa7, 0xa7, 0xa7);
const ROW_HEIGHT: f32 = 28.0;
const ART_SIZE: f32 = 56.0;

/// Messages from background tasks to the UI thread.
enum Msg {
    SignInUrl(String),
    SignedIn(Arc<WebApi>, Option<String>),
    SignInFailed(String),
    NewRefreshToken(String),
    UserName(String),
    Playlists(Result<Vec<Playlist>, String>),
    TracksPage {
        view: u64,
        tracks: Vec<Track>,
    },
    TracksDone {
        view: u64,
        error: Option<String>,
    },
    Search {
        view: u64,
        result: Result<SearchResults, String>,
    },
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

enum ViewKind {
    Tracks(TrackSource),
    Search {
        query: String,
        results: Option<SearchResults>,
    },
}

struct View {
    id: u64,
    kind: ViewKind,
    tracks: Vec<Track>,
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

    api: Option<Arc<WebApi>>,
    user_name: String,
    playlists: Vec<Playlist>,
    playlists_loading: bool,
    view: View,
    next_view_id: u64,
    search_input: String,
    notice: Option<String>,

    player: Option<PlayerCtl>,
    player_state: PlayerState,
    queue: Vec<Track>,
    queue_pos: usize,
    now: NowPlaying,
    /// Seek bar position while the user is dragging it.
    seek_drag: Option<f32>,
    skipped_in_a_row: usize,
    cover: Option<(String, TextureHandle)>,
    cover_requested: Option<String>,
    /// The browser sign-in that is waiting for the user, so it can be cancelled.
    sign_in_task: Option<tokio::task::JoinHandle<()>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Runtime) -> Self {
        setup_style(&cc.egui_ctx);
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

        let mut app = Self {
            rt,
            http,
            rx,
            outbox,
            screen: Screen::Setup {
                input: String::new(),
                error: None,
            },
            config,
            api: None,
            user_name: String::new(),
            playlists: Vec::new(),
            playlists_loading: false,
            view: View {
                id: 0,
                kind: ViewKind::Tracks(TrackSource::LikedSongs),
                tracks: Vec::new(),
                loading: false,
                error: None,
            },
            next_view_id: 1,
            search_input: String::new(),
            notice: None,
            player: None,
            player_state: PlayerState::Off,
            queue: Vec::new(),
            queue_pos: 0,
            now: NowPlaying::default(),
            seek_drag: None,
            skipped_in_a_row: 0,
            cover: None,
            cover_requested: None,
            sign_in_task: None,
        };

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

    fn on_signed_in(&mut self, api: Arc<WebApi>, refresh_token: Option<String>) {
        if refresh_token.is_some() {
            self.config.refresh_token = refresh_token;
            self.config.save();
        }
        self.api = Some(api.clone());
        self.screen = Screen::Main;

        let outbox = self.outbox.clone();
        let api2 = api.clone();
        self.rt.spawn(async move {
            if let Ok(name) = api2.user_name().await {
                outbox.send(Msg::UserName(name));
            }
        });
        self.reload_playlists();
        self.open(ViewKind::Tracks(TrackSource::LikedSongs));
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
        self.queue.clear();
        self.now = NowPlaying::default();
        self.cover = None;
        self.user_name.clear();
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

    /// Switches the main area to a new view and starts loading it.
    fn open(&mut self, kind: ViewKind) {
        let Some(api) = self.api.clone() else { return };
        let id = self.next_view_id;
        self.next_view_id += 1;
        let outbox = self.outbox.clone();

        match &kind {
            ViewKind::Tracks(source) => {
                let source = source.clone();
                let player = self.player.clone();
                self.rt.spawn(async move {
                    let mut got_any = false;
                    let result = api
                        .tracks(&source, |tracks| {
                            got_any = true;
                            outbox.send(Msg::TracksPage { view: id, tracks });
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
                                        outbox.send(Msg::TracksPage { view: id, tracks });
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
                    outbox.send(Msg::TracksDone { view: id, error });
                });
            }
            ViewKind::Search { query, .. } => {
                let query = query.clone();
                self.rt.spawn(async move {
                    let result = api.search(&query).await.map_err(|e| format!("{e:#}"));
                    outbox.send(Msg::Search { view: id, result });
                });
            }
        }

        self.view = View {
            id,
            kind,
            tracks: Vec::new(),
            loading: true,
            error: None,
        };
    }

    // ------------------------------------------------------------- playback

    /// Plays `tracks[index]` and makes `tracks` the queue for next/previous.
    fn play_list(&mut self, tracks: Vec<Track>, index: usize) {
        self.queue = tracks;
        self.skipped_in_a_row = 0;
        self.play_at(index, true);
    }

    /// Plays the first playable track at or after (or before, if `forward` is
    /// false) `index` in the queue.
    fn play_at(&mut self, index: usize, forward: bool) -> bool {
        let Some(player) = &self.player else {
            return false;
        };
        if !matches!(self.player_state, PlayerState::Ready) {
            self.notice = Some("The player is not connected yet. Please wait a moment.".into());
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

    fn next(&mut self) {
        if !self.play_at(self.queue_pos + 1, true) {
            // End of the list.
            if let Some(p) = &self.player {
                p.stop();
            }
            self.now.playing = false;
            self.now.loading = false;
            self.now.position_ms = 0;
            self.now.since = None;
        }
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
            // Shrink big covers so they take little memory.
            let img = img.thumbnail(128, 128).to_rgba8();
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
                let next = self
                    .queue
                    .iter()
                    .skip(self.queue_pos + 1)
                    .find(|t| t.playable);
                if let (Some(p), Some(t)) = (&self.player, next) {
                    p.preload(&t.uri);
                }
            }
            PlayerMsg::EndOfTrack { uri } if is_current(&uri) => self.next(),
            PlayerMsg::Unavailable { uri } if is_current(&uri) => {
                self.skipped_in_a_row += 1;
                if self.skipped_in_a_row > 5 {
                    self.notice = Some(
                        "Several tracks in a row could not be played. Playback stopped.".into(),
                    );
                    self.now.loading = false;
                } else {
                    let name = self
                        .now
                        .track
                        .as_ref()
                        .map(|t| t.name.clone())
                        .unwrap_or_default();
                    self.notice = Some(format!("\"{name}\" is not available, skipped it."));
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
                        Err(e) => self.notice = Some(format!("Could not load playlists: {e}")),
                    }
                }
                Msg::TracksPage { view, tracks } if view == self.view.id => {
                    self.view.tracks.extend(tracks);
                }
                Msg::TracksDone { view, error } if view == self.view.id => {
                    self.view.loading = false;
                    self.view.error = error;
                }
                Msg::Search { view, result } if view == self.view.id => {
                    self.view.loading = false;
                    match result {
                        Ok(r) => {
                            if let ViewKind::Search { results, .. } = &mut self.view.kind {
                                *results = Some(r);
                            }
                        }
                        Err(e) => self.view.error = Some(e),
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
                _ => {} // results for a view that is no longer open
            }
        }
    }

    // ------------------------------------------------------------------ UI

    fn setup_screen(&mut self, ui: &mut egui::Ui) {
        let Screen::Setup { input, error } = &mut self.screen else {
            return;
        };
        let mut submit = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.heading(RichText::new("Welcome to lite-spotify").size(26.0));
            });
            ui.add_space(16.0);
            ui.label("lite-spotify talks to Spotify through your own (free) Spotify developer app. One-time setup:");
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("1. Open");
                ui.hyperlink("https://developer.spotify.com/dashboard");
                ui.label("and log in with your Spotify account.");
            });
            ui.label("2. Click \"Create app\". Any name and description will do.");
            ui.horizontal(|ui| {
                ui.label("3. Under \"Redirect URIs\" add exactly:");
                ui.code(auth::WEB_REDIRECT_URI);
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(auth::WEB_REDIRECT_URI.to_string());
                }
            });
            ui.label("4. Tick \"Web API\", accept the terms and save.");
            ui.label("5. Copy the app's \"Client ID\" and paste it here:");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(input)
                        .hint_text("Client ID (32 letters and numbers)")
                        .desired_width(340.0),
                );
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    submit = true;
                }
                if ui.button("Save and sign in").clicked() {
                    submit = true;
                }
            });
            if let Some(e) = error {
                ui.colored_label(Color32::LIGHT_RED, e.as_str());
            }
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!("Settings are saved in {}", config::app_dir().display()))
                    .color(DIM)
                    .small(),
            );
        });

        if submit {
            let id = input.trim().to_string();
            if id.len() == 32 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
                self.config.client_id = id;
                self.config.refresh_token = None;
                self.config.save();
                self.start_browser_sign_in();
            } else {
                *error = Some(
                    "That does not look like a Client ID. It should be 32 letters and numbers."
                        .into(),
                );
            }
        }
    }

    fn sign_in_screen(&mut self, ui: &mut egui::Ui) {
        let Screen::SignIn { url, busy, error } = &self.screen else {
            return;
        };
        let (url, busy, error) = (url.clone(), *busy, error.clone());
        let mut sign_in = false;
        let mut change_id = false;
        let mut cancel = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.heading(RichText::new("lite-spotify").size(30.0).color(GREEN));
                ui.add_space(20.0);
                if busy {
                    ui.spinner();
                    if url.is_some() {
                        ui.label("Finish signing in in your browser, then come back here.");
                    } else {
                        ui.label("Signing in…");
                    }
                    if let Some(url) = &url {
                        ui.add_space(8.0);
                        ui.hyperlink_to("Browser did not open? Click here.", url);
                        ui.add_space(12.0);
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    }
                } else {
                    if let Some(e) = &error {
                        ui.colored_label(Color32::LIGHT_RED, e);
                        ui.add_space(12.0);
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("  Sign in with Spotify  ")
                                    .size(18.0)
                                    .color(Color32::BLACK),
                            )
                            .fill(GREEN),
                        )
                        .clicked()
                    {
                        sign_in = true;
                    }
                    ui.add_space(20.0);
                    if ui.small_button("Use a different Client ID").clicked() {
                        change_id = true;
                    }
                }
            });
        });
        if sign_in {
            self.start_browser_sign_in();
        }
        if cancel {
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
        if change_id {
            self.screen = Screen::Setup {
                input: self.config.client_id.clone(),
                error: None,
            };
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.search_input)
                        .hint_text("🔍 Search songs, albums, playlists")
                        .desired_width(320.0),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if (enter || ui.button("Search").clicked()) && !self.search_input.trim().is_empty()
                {
                    let query = self.search_input.trim().to_string();
                    self.open(ViewKind::Search {
                        query,
                        results: None,
                    });
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button("⚙", |ui| {
                        if ui.button("Reload playlists").clicked() {
                            self.reload_playlists();
                        }
                        if ui.button("Open settings folder").clicked() {
                            let _ = open::that_detached(config::app_dir());
                        }
                        if ui.button("Sign out").clicked() {
                            self.sign_out();
                        }
                    });
                    if !self.user_name.is_empty() {
                        ui.label(RichText::new(&self.user_name).color(DIM));
                    }
                });
            });
            if let Some(notice) = self.notice.clone() {
                ui.horizontal(|ui| {
                    ui.colored_label(Color32::from_rgb(0xff, 0xc8, 0x6b), notice);
                    if ui.small_button("✖").clicked() {
                        self.notice = None;
                    }
                });
            }
            ui.add_space(4.0);
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let mut open: Option<ViewKind> = None;
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(220.0)
            .size_range(160.0..=400.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                let liked = matches!(&self.view.kind, ViewKind::Tracks(TrackSource::LikedSongs));
                if ui
                    .selectable_label(liked, RichText::new("❤ Liked Songs").strong())
                    .clicked()
                {
                    open = Some(ViewKind::Tracks(TrackSource::LikedSongs));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Playlists").color(DIM));
                    if self.playlists_loading {
                        ui.spinner();
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        for p in &self.playlists {
                            let selected = matches!(&self.view.kind,
                            ViewKind::Tracks(TrackSource::Playlist { id, .. }) if *id == p.id);
                            let label = ui
                                .add(egui::Button::selectable(selected, p.name.as_str()).truncate())
                                .on_hover_text(format!(
                                    "{} · by {} · {} tracks",
                                    p.name, p.owner, p.total
                                ));
                            if label.clicked() {
                                open = Some(ViewKind::Tracks(TrackSource::Playlist {
                                    id: p.id.clone(),
                                    name: p.name.clone(),
                                }));
                            }
                        }
                    });
            });
        if let Some(kind) = open {
            self.open(kind);
        }
    }

    fn now_playing_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("now_playing")
            .exact_size(ART_SIZE + 24.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x18, 0x18, 0x18))
                    .inner_margin(12.0),
            )
            .show(ui, |ui| {
                let width = ui.available_width();
                let side = (width * 0.3).clamp(180.0, 360.0);
                ui.horizontal_centered(|ui| {
                    // Left: cover, title and artist.
                    ui.allocate_ui_with_layout(
                        Vec2::new(side, ART_SIZE),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            match &self.cover {
                                Some((_, tex)) if self.now.track.is_some() => {
                                    ui.add(
                                        egui::Image::new((tex.id(), Vec2::splat(ART_SIZE)))
                                            .corner_radius(4.0),
                                    );
                                }
                                _ => {
                                    let (rect, _) = ui
                                        .allocate_exact_size(Vec2::splat(ART_SIZE), Sense::hover());
                                    ui.painter().rect_filled(
                                        rect,
                                        CornerRadius::same(4),
                                        Color32::from_gray(40),
                                    );
                                }
                            }
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width());
                                ui.add_space(8.0);
                                if let Some(t) = &self.now.track {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&t.name).strong().color(Color32::WHITE),
                                        )
                                        .truncate(),
                                    );
                                    ui.add(
                                        egui::Label::new(RichText::new(&t.artists).color(DIM))
                                            .truncate(),
                                    );
                                } else {
                                    ui.label(RichText::new(self.player_status_text()).color(DIM));
                                }
                            });
                        },
                    );

                    // Middle: buttons and seek bar.
                    let middle = (ui.available_width() - side).max(200.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(middle, ART_SIZE),
                        Layout::top_down(Align::Center),
                        |ui| {
                            self.transport_controls(ui);
                            self.seek_bar(ui);
                        },
                    );

                    // Right: volume.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().slider_width =
                            (ui.available_width() - 40.0).clamp(60.0, 120.0);
                        let response = ui.add(
                            egui::Slider::new(&mut self.config.volume, 0.0..=1.0).show_value(false),
                        );
                        if response.changed()
                            && let Some(p) = &self.player
                        {
                            p.set_volume(self.config.volume);
                        }
                        if response.drag_stopped() || (response.changed() && !response.dragged()) {
                            self.config.save();
                        }
                        ui.label(if self.config.volume == 0.0 {
                            "🔇"
                        } else {
                            "🔊"
                        });
                    });
                });
            });
    }

    fn player_status_text(&self) -> String {
        match &self.player_state {
            PlayerState::Off => "".into(),
            PlayerState::Connecting => "Connecting the player…".into(),
            PlayerState::BrowserLogin(_) => "Approve playback in your browser".into(),
            PlayerState::Ready => "Double-click a song to play it".into(),
            PlayerState::Failed(_) => "Player is not connected".into(),
        }
    }

    fn transport_controls(&mut self, ui: &mut egui::Ui) {
        let ready = matches!(self.player_state, PlayerState::Ready);
        let has_track = self.now.track.is_some();
        ui.horizontal(|ui| {
            // Centre the three buttons in the available width.
            let buttons_width = 3.0 * 36.0 + 2.0 * ui.spacing().item_spacing.x;
            ui.add_space(((ui.available_width() - buttons_width) / 2.0).max(0.0));
            let big = |s: &str| RichText::new(s).size(18.0);
            if ui
                .add_enabled(
                    ready && has_track,
                    egui::Button::new(big("⏮")).min_size(Vec2::new(36.0, 24.0)),
                )
                .on_hover_text("Previous")
                .clicked()
            {
                self.previous();
            }
            let icon = if self.now.playing { "⏸" } else { "▶" };
            if ui
                .add_enabled(
                    ready && has_track,
                    egui::Button::new(big(icon)).min_size(Vec2::new(36.0, 24.0)),
                )
                .on_hover_text("Play/pause (space)")
                .clicked()
            {
                self.toggle_play();
            }
            if ui
                .add_enabled(
                    ready && has_track,
                    egui::Button::new(big("⏭")).min_size(Vec2::new(36.0, 24.0)),
                )
                .on_hover_text("Next")
                .clicked()
            {
                self.next();
            }
        });
    }

    fn seek_bar(&mut self, ui: &mut egui::Ui) {
        let duration_ms = self.now.track.as_ref().map(|t| t.duration_ms).unwrap_or(0);
        ui.horizontal(|ui| {
            let pos_ms = self.now.position_ms();
            let shown_ms = self
                .seek_drag
                .map(|s| (s * 1000.0) as u32)
                .unwrap_or(pos_ms);
            let time_width = 44.0;
            ui.add_sized(
                [time_width, 16.0],
                egui::Label::new(RichText::new(format_time(shown_ms)).small().color(DIM)),
            );
            ui.spacing_mut().slider_width = (ui.available_width() - time_width - 16.0).max(60.0);
            let mut value = shown_ms as f32 / 1000.0;
            let enabled = duration_ms > 0 && matches!(self.player_state, PlayerState::Ready);
            let response = ui.add_enabled(
                enabled,
                egui::Slider::new(&mut value, 0.0..=(duration_ms as f32 / 1000.0).max(0.1))
                    .show_value(false),
            );
            if response.changed() {
                self.seek_drag = Some(value);
            }
            // Seek once the user lets go of the handle (or after a plain click).
            if let Some(target) = self.seek_drag
                && !response.dragged()
            {
                self.seek_drag = None;
                self.seek((target * 1000.0) as u32);
            }
            ui.add_sized(
                [time_width, 16.0],
                egui::Label::new(RichText::new(format_time(duration_ms)).small().color(DIM)),
            );
        });
    }

    fn player_banner(&mut self, ui: &mut egui::Ui) {
        let mut retry = false;
        match &self.player_state {
            PlayerState::BrowserLogin(url) => {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(GREEN, "One more step: a browser tab asks you to allow playback (this is Spotify's own player login).");
                    ui.hyperlink_to("Open it again", url);
                });
            }
            PlayerState::Failed(e) => {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(Color32::LIGHT_RED, format!("Player: {e}"));
                    if ui.button("Retry").clicked() {
                        retry = true;
                    }
                });
            }
            _ => {}
        }
        if retry {
            self.start_player();
        }
    }

    fn main_area(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            self.player_banner(ui);

            let title = match &self.view.kind {
                ViewKind::Tracks(TrackSource::LikedSongs) => "Liked Songs".to_string(),
                ViewKind::Tracks(TrackSource::Playlist { name, .. }) => name.clone(),
                ViewKind::Tracks(TrackSource::Album { name, .. }) => name.clone(),
                ViewKind::Search { query, .. } => format!("Search: {query}"),
            };
            ui.horizontal(|ui| {
                ui.heading(RichText::new(title).size(22.0).strong());
                if let ViewKind::Tracks(_) = self.view.kind {
                    ui.label(RichText::new(format!("{} songs", self.view.tracks.len())).color(DIM));
                }
                if self.view.loading {
                    ui.spinner();
                }
            });
            if let Some(e) = &self.view.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.add_space(4.0);

            if matches!(self.view.kind, ViewKind::Search { .. }) {
                self.search_results(ui);
            } else {
                header_row(ui);
                let current = self.now.track.as_ref().map(|t| t.uri.as_str());
                let mut play = None;
                // Rows sit directly below each other; the hover highlight separates them.
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical().auto_shrink(false).show_rows(
                    ui,
                    ROW_HEIGHT,
                    self.view.tracks.len(),
                    |ui, range| {
                        for i in range {
                            let t = &self.view.tracks[i];
                            if track_row(ui, i + 1, t, current == Some(t.uri.as_str())) {
                                play = Some(i);
                            }
                        }
                    },
                );
                if let Some(i) = play {
                    self.play_list(self.view.tracks.clone(), i);
                }
            }
        });
    }

    fn search_results(&mut self, ui: &mut egui::Ui) {
        let ViewKind::Search {
            results: Some(results),
            ..
        } = &self.view.kind
        else {
            return;
        };
        let current = self.now.track.as_ref().map(|t| t.uri.as_str());
        let mut play = None;
        let mut open = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.label(RichText::new("Songs").size(17.0).strong());
                if results.tracks.is_empty() {
                    ui.label(RichText::new("No songs found").color(DIM));
                } else {
                    header_row(ui);
                }
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, t) in results.tracks.iter().enumerate() {
                        if track_row(ui, i + 1, t, current == Some(t.uri.as_str())) {
                            play = Some(i);
                        }
                    }
                });
                ui.add_space(12.0);
                ui.label(RichText::new("Albums").size(17.0).strong());
                for a in &results.albums {
                    if link_row(ui, &a.name, &a.artists) {
                        open = Some(album_view(a));
                    }
                }
                ui.add_space(12.0);
                ui.label(RichText::new("Playlists").size(17.0).strong());
                for p in &results.playlists {
                    if link_row(ui, &p.name, &format!("by {}", p.owner)) {
                        open = Some(ViewKind::Tracks(TrackSource::Playlist {
                            id: p.id.clone(),
                            name: p.name.clone(),
                        }));
                    }
                }
            });
        if let Some(i) = play {
            let tracks = results.tracks.clone();
            self.play_list(tracks, i);
        }
        if let Some(kind) = open {
            self.open(kind);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_messages(&ctx);

        match self.screen {
            Screen::Setup { .. } => self.setup_screen(ui),
            Screen::SignIn { .. } => self.sign_in_screen(ui),
            Screen::Main => {
                // Space toggles play/pause, unless the user is typing. The key is consumed
                // so a focused button does not also react to it.
                if !ctx.egui_wants_keyboard_input()
                    && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Space))
                {
                    self.toggle_play();
                }
                self.top_bar(ui);
                self.now_playing_bar(ui);
                self.sidebar(ui);
                self.main_area(ui);
                // Keep the seek bar moving; otherwise only redraw when something happens.
                if self.now.playing {
                    ctx.request_repaint_after(Duration::from_millis(500));
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.config.save();
    }
}

fn album_view(a: &Album) -> ViewKind {
    ViewKind::Tracks(TrackSource::Album {
        id: a.id.clone(),
        name: a.name.clone(),
    })
}

fn is_forbidden(e: &anyhow::Error) -> bool {
    e.downcast_ref::<api::ApiError>().is_some_and(|e| {
        e.status == reqwest::StatusCode::FORBIDDEN || e.status == reqwest::StatusCode::NOT_FOUND
    })
}

fn format_time(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// x offsets and widths of the track list columns.
fn columns(width: f32) -> [(f32, f32); 5] {
    let num = 40.0;
    let dur = 56.0;
    let rest = (width - num - dur).max(100.0);
    let title = rest * 0.42;
    let artist = rest * 0.30;
    let album = rest - title - artist;
    [
        (0.0, num),
        (num, title),
        (num + title, artist),
        (num + title + artist, album),
        (num + title + artist + album, dur),
    ]
}

fn header_row(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
    let cols = columns(rect.width());
    let font = egui::FontId::proportional(12.0);
    for ((x, _), text) in cols.iter().zip(["#", "Title", "Artist", "Album", "Time"]) {
        ui.painter().text(
            rect.left_center() + Vec2::new(x + 6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            font.clone(),
            DIM,
        );
    }
    ui.painter()
        .hline(rect.x_range(), rect.bottom(), (1.0, Color32::from_gray(50)));
}

/// Draws one song row. Returns true when the user wants to play it.
fn track_row(ui: &mut egui::Ui, number: usize, t: &Track, is_current: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), Color32::from_gray(38));
    }
    let color = if !t.playable {
        Color32::from_gray(90)
    } else if is_current {
        GREEN
    } else {
        Color32::from_gray(225)
    };
    let cols = columns(rect.width());
    let number_text = if hovered && t.playable {
        "▶".to_string()
    } else {
        number.to_string()
    };
    let texts = [
        number_text,
        t.name.clone(),
        t.artists.clone(),
        t.album.clone(),
        format_time(t.duration_ms),
    ];
    for (i, ((x, w), text)) in cols.iter().zip(texts).enumerate() {
        let cell = egui::Rect::from_min_size(
            rect.min + Vec2::new(x + 6.0, 0.0),
            Vec2::new(w - 10.0, ROW_HEIGHT),
        );
        let text_color = if i >= 2 && t.playable && !is_current {
            DIM
        } else {
            color
        };
        let label = egui::Label::new(RichText::new(text).color(text_color))
            .truncate()
            .selectable(false);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(cell)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| ui.add(label),
        );
    }
    if !t.playable {
        response.on_hover_text("This song is not available");
        return false;
    }
    let clicked_number = response.clicked()
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| p.x < rect.left() + cols[1].0);
    response.double_clicked() || clicked_number
}

/// A clickable line for an album or playlist in search results.
fn link_row(ui: &mut egui::Ui, name: &str, detail: &str) -> bool {
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::Button::new(RichText::new(name).color(Color32::WHITE))
                .frame(false)
                .truncate(),
        );
        ui.label(RichText::new(detail).color(DIM));
        r.clicked()
    })
    .inner
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(0x12, 0x12, 0x12);
    visuals.window_fill = Color32::from_rgb(0x1e, 0x1e, 0x1e);
    visuals.extreme_bg_color = Color32::from_rgb(0x24, 0x24, 0x24);
    visuals.selection.bg_fill = Color32::from_rgb(0x1d, 0x6b, 0x38);
    visuals.hyperlink_color = GREEN;
    visuals.slider_trailing_fill = true;
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.selectable_labels = false;
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        for (text_style, font) in style.text_styles.iter_mut() {
            font.size = match text_style {
                egui::TextStyle::Small => 11.5,
                egui::TextStyle::Heading => 20.0,
                _ => 14.0,
            };
        }
    });
}
