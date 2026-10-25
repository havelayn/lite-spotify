//! The window: sign-in screens, sidebar, track lists and the now-playing bar.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Key, Layout, Margin, Rect, RichText, Sense, Stroke,
    TextureHandle, Ui, Vec2, pos2, vec2,
};

use crate::api::{self, Album, Playlist, SearchResults, Track, TrackSource, WebApi};
use crate::auth;
use crate::config::{self, Config};
use crate::player::{PlayerCtl, PlayerMsg};
use crate::theme::{self, BG, CARD, ERROR, GREEN, HOVER, MUTED, RAISED, SUBDUED, TEXT};
use crate::widgets::{
    self, Icon, PillStyle, TRACK_HEADER_HEIGHT, TRACK_ROW_HEIGHT, Tile, format_time,
};

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
    /// A short message shown at the bottom of the window, and when it appeared.
    notice: Option<(String, Instant)>,

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
    /// The list the queue was started from, to highlight it in the library.
    playing_source: Option<TrackSource>,
    volume_before_mute: f32,

    liked_tile: TextureHandle,
    /// Background fade at the top of the main area, and the colour it was made for.
    header_fade: Option<(Color32, TextureHandle)>,
    /// When the redirect URI was copied, to show "Copied" for a moment.
    copied_at: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Runtime) -> Self {
        theme::setup(&cc.egui_ctx);
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
            playing_source: None,
            volume_before_mute: 0.5,
            liked_tile: widgets::gradient_texture(&cc.egui_ctx, "liked", [64, 64], |x, y| {
                let a = Color32::from_rgb(0x45, 0x0a, 0xf5);
                let b = Color32::from_rgb(0xc4, 0xef, 0xd9);
                widgets::lerp_color(a, b, (x + y) / 2.0)
            }),
            header_fade: None,
            copied_at: None,
        };

        #[cfg(debug_assertions)]
        if std::env::var_os("LITE_SPOTIFY_DEMO").is_some() {
            app.fill_demo_data();
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
        self.playing_source = None;
        self.now = NowPlaying::default();
        self.cover = None;
        self.user_name.clear();
        self.screen = Screen::SignIn {
            url: None,
            busy: false,
            error: None,
        };
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
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
    fn play_list(&mut self, tracks: Vec<Track>, index: usize, source: Option<TrackSource>) {
        self.playing_source = source;
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
                    self.notify("Several tracks in a row could not be played. Playback stopped.");
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

    /// A dark card in the middle of the window, used by the setup and sign-in screens.
    fn centered_card(ui: &mut Ui, width: f32, add_contents: impl FnOnce(&mut Ui)) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let width = width.min(ui.available_width() - 32.0);
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.06).clamp(16.0, 80.0));
                        Frame::new()
                            .fill(CARD)
                            .corner_radius(12)
                            .inner_margin(32)
                            .show(ui, |ui| {
                                ui.set_width(width - 64.0);
                                ui.with_layout(Layout::top_down(Align::Min), add_contents);
                            });
                        ui.add_space(24.0);
                    });
                });
            });
    }

    fn setup_screen(&mut self, ui: &mut Ui) {
        let copied = self
            .copied_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
        if copied {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        let Screen::Setup { input, error } = &mut self.screen else {
            return;
        };
        let mut submit = false;
        let mut copy = false;
        Self::centered_card(ui, 600.0, |ui| {
            widgets::logo(ui, 22.0);
            ui.add_space(14.0);
            ui.label(
                RichText::new("Welcome to lite-spotify")
                    .font(theme::bold(28.0))
                    .color(TEXT),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "lite-spotify connects to Spotify through your own free developer app. \
                     This takes about three minutes and only has to be done once.",
                )
                .color(SUBDUED),
            );
            ui.add_space(20.0);

            step(ui, 1, |ui| {
                ui.label("Open");
                ui.hyperlink_to(
                    "developer.spotify.com/dashboard",
                    "https://developer.spotify.com/dashboard",
                );
                ui.label("and log in.");
            });
            step(ui, 2, |ui| {
                ui.label("Click \"Create app\". Any name and description will do.");
            });
            step(ui, 3, |ui| {
                ui.label("Under \"Redirect URIs\", add exactly this address:");
            });
            ui.horizontal(|ui| {
                ui.add_space(36.0);
                Frame::new()
                    .fill(RAISED)
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(auth::WEB_REDIRECT_URI)
                                .monospace()
                                .color(TEXT),
                        );
                    });
                let label = if copied { "Copied" } else { "Copy" };
                if widgets::pill_button(ui, label, PillStyle::Outline, 13.0, false).clicked() {
                    copy = true;
                }
            });
            ui.add_space(4.0);
            step(ui, 4, |ui| {
                ui.label("Tick \"Web API\", accept the terms and click Save.");
            });
            step(ui, 5, |ui| {
                ui.label("Open the app's Settings, copy the Client ID and paste it below.");
            });
            ui.add_space(12.0);

            let edit = ui.add(
                egui::TextEdit::singleline(input)
                    .hint_text("Client ID")
                    .font(theme::regular(16.0))
                    .margin(vec2(14.0, 12.0))
                    .desired_width(f32::INFINITY),
            );
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            if let Some(e) = error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(ERROR));
            }
            ui.add_space(16.0);
            if widgets::pill_button(ui, "Save and sign in", PillStyle::Primary, 16.0, true)
                .clicked()
            {
                submit = true;
            }
            ui.add_space(16.0);
            ui.label(
                RichText::new(format!(
                    "Settings are saved in {}",
                    config::app_dir().display()
                ))
                .color(MUTED)
                .small(),
            );
        });

        if copy {
            ui.ctx().copy_text(auth::WEB_REDIRECT_URI.to_string());
            self.copied_at = Some(Instant::now());
        }
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

    fn sign_in_screen(&mut self, ui: &mut Ui) {
        let Screen::SignIn { url, busy, error } = &self.screen else {
            return;
        };
        let (url, busy, error) = (url.clone(), *busy, error.clone());
        let mut sign_in = false;
        let mut change_id = false;
        let mut cancel = false;
        Self::centered_card(ui, 440.0, |ui| {
            ui.vertical_centered(|ui| {
                widgets::logo(ui, 32.0);
                ui.add_space(14.0);
                ui.label(
                    RichText::new("lite-spotify")
                        .font(theme::bold(32.0))
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("A small player for your Spotify Premium account").color(SUBDUED),
                );
                ui.add_space(28.0);
                if busy {
                    ui.add(egui::Spinner::new().size(28.0).color(GREEN));
                    ui.add_space(12.0);
                    if let Some(url) = &url {
                        ui.label(RichText::new("Finish signing in in your browser,").color(TEXT));
                        ui.label(RichText::new("then come back here.").color(TEXT));
                        ui.add_space(12.0);
                        ui.hyperlink_to("Browser did not open? Click here.", url);
                        ui.add_space(20.0);
                        if widgets::pill_button(ui, "Cancel", PillStyle::Outline, 14.0, false)
                            .clicked()
                        {
                            cancel = true;
                        }
                    } else {
                        ui.label(RichText::new("Signing in…").color(TEXT));
                    }
                } else {
                    if let Some(e) = &error {
                        ui.label(RichText::new(e).color(ERROR));
                        ui.add_space(16.0);
                    }
                    if widgets::pill_button(
                        ui,
                        "Sign in with Spotify",
                        PillStyle::Primary,
                        16.0,
                        true,
                    )
                    .clicked()
                    {
                        sign_in = true;
                    }
                    ui.add_space(16.0);
                    let other = ui.add(
                        egui::Button::new(
                            RichText::new("Use a different Client ID").color(SUBDUED),
                        )
                        .frame(false),
                    );
                    if other.clicked() {
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

    fn main_screen(&mut self, ui: &mut Ui) {
        egui::Panel::top("top")
            .exact_size(64.0)
            .show_separator_line(false)
            .frame(Frame::new().fill(BG))
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("player")
            .exact_size(84.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| self.player_bar(ui));
        egui::Panel::left("library")
            .resizable(true)
            .default_size(300.0)
            .size_range(220.0..=440.0)
            .show_separator_line(false)
            .frame(Frame::new().fill(BG).inner_margin(Margin {
                left: 8,
                right: 0,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| self.library(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin {
                left: 8,
                right: 8,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| self.main_card(ui));
        self.toast(ui.ctx());
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();

        // Left: logo and name.
        let logo =
            Rect::from_center_size(pos2(full.left() + 32.0, full.center().y), Vec2::splat(32.0));
        widgets::paint_logo(ui.painter(), logo.center(), 16.0);
        ui.painter().text(
            pos2(logo.right() + 10.0, full.center().y),
            egui::Align2::LEFT_CENTER,
            "lite-spotify",
            theme::bold(17.0),
            TEXT,
        );

        // Middle: the search box.
        let width = (full.width() * 0.4).clamp(260.0, 480.0);
        let search = Rect::from_center_size(full.center(), vec2(width, 44.0));
        if self.search_box(ui, search) {
            let query = self.search_input.trim().to_string();
            if !query.is_empty() {
                self.open(ViewKind::Search {
                    query,
                    results: None,
                });
            }
        }

        // Right: avatar with the menu.
        let avatar = Rect::from_center_size(
            pos2(full.right() - 36.0, full.center().y),
            Vec2::splat(40.0),
        );
        let response = ui.interact(avatar, ui.scope_id().with("avatar"), Sense::click());
        let p = ui.painter();
        p.circle_filled(
            avatar.center(),
            20.0,
            if response.hovered() { HOVER } else { RAISED },
        );
        p.circle_filled(avatar.center(), 14.0, Color32::from_rgb(0xf5, 0x73, 0x3a));
        let initial = self
            .user_name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        p.text(
            avatar.center(),
            egui::Align2::CENTER_CENTER,
            initial,
            theme::bold(14.0),
            Color32::BLACK,
        );
        let response = response.on_hover_text(&self.user_name);
        egui::Popup::menu(&response).show(|ui| {
            ui.set_min_width(200.0);
            if !self.user_name.is_empty() {
                ui.label(
                    RichText::new(&self.user_name)
                        .font(theme::bold(14.0))
                        .color(TEXT),
                );
                ui.separator();
            }
            if ui.button("Reload playlists").clicked() {
                self.reload_playlists();
            }
            if ui.button("Open settings folder").clicked() {
                let _ = open::that_detached(config::app_dir());
            }
            ui.separator();
            if ui.button("Sign out").clicked() {
                self.sign_out();
            }
            ui.separator();
            ui.label(
                RichText::new(concat!("lite-spotify ", env!("CARGO_PKG_VERSION")))
                    .small()
                    .color(MUTED),
            );
        });
    }

    /// The rounded search box. Returns true when the user presses Enter.
    fn search_box(&mut self, ui: &mut Ui, rect: Rect) -> bool {
        let id = egui::Id::unique("search");
        let focused = ui.memory(|m| m.has_focus(id));
        let background = ui.interact(rect, id.with("bg"), Sense::click());
        if background.clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let fill = if focused || background.hovered() {
            HOVER
        } else {
            RAISED
        };
        let stroke = if focused {
            Stroke::new(1.5, TEXT)
        } else {
            Stroke::NONE
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(255),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );
        let icon =
            Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), Vec2::splat(20.0));
        widgets::paint_icon(
            ui.painter(),
            icon,
            Icon::Search,
            if focused { TEXT } else { SUBDUED },
        );

        let text_rect = Rect::from_min_max(
            pos2(rect.left() + 44.0, rect.top()),
            pos2(rect.right() - 44.0, rect.bottom()),
        );
        let mut submitted = false;
        widgets::place(
            ui,
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.search_input)
                        .id(id)
                        .frame(Frame::NONE)
                        .font(theme::regular(15.0))
                        .hint_text("What do you want to play?")
                        .desired_width(f32::INFINITY),
                );
                submitted = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            },
        );
        if !self.search_input.is_empty() {
            let clear = Rect::from_center_size(
                pos2(rect.right() - 24.0, rect.center().y),
                Vec2::splat(24.0),
            );
            let response = widgets::place(ui, egui::UiBuilder::new().max_rect(clear), |ui| {
                widgets::icon_button(ui, Icon::Close, 14.0, true)
            });
            if response.on_hover_text("Clear").clicked() {
                self.search_input.clear();
            }
        }
        submitted
    }

    fn library(&mut self, ui: &mut Ui) {
        let mut open = None;
        Frame::new()
            .fill(CARD)
            .corner_radius(8)
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Your Library")
                            .font(theme::bold(16.0))
                            .color(TEXT),
                    );
                    if self.playlists_loading {
                        ui.add(egui::Spinner::new().size(14.0).color(SUBDUED));
                    }
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let liked = TrackSource::LikedSongs;
                        let selected =
                            matches!(&self.view.kind, ViewKind::Tracks(s) if *s == liked);
                        let playing = self.playing_source.as_ref() == Some(&liked);
                        if widgets::library_row(
                            ui,
                            Tile::Liked(&self.liked_tile),
                            "Liked Songs",
                            "Playlist",
                            selected,
                            playing,
                        )
                        .clicked()
                        {
                            open = Some(ViewKind::Tracks(liked));
                        }
                        for p in &self.playlists {
                            let source = TrackSource::Playlist {
                                id: p.id.clone(),
                                name: p.name.clone(),
                            };
                            let selected = matches!(&self.view.kind,
                                ViewKind::Tracks(TrackSource::Playlist { id, .. }) if *id == p.id);
                            let playing = matches!(&self.playing_source,
                                Some(TrackSource::Playlist { id, .. }) if *id == p.id);
                            let subtitle = format!("Playlist · {}", p.owner);
                            if widgets::library_row(
                                ui,
                                Tile::Playlist,
                                &p.name,
                                &subtitle,
                                selected,
                                playing,
                            )
                            .on_hover_text(format!("{} · {} songs", p.name, p.total))
                            .clicked()
                            {
                                open = Some(ViewKind::Tracks(source));
                            }
                        }
                    });
            });
        if let Some(kind) = open {
            self.open(kind);
        }
    }

    fn player_bar(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let side = (full.width() * 0.3).clamp(200.0, 380.0);
        let left = Rect::from_min_max(full.min, pos2(full.left() + side, full.bottom()));
        let right = Rect::from_min_max(pos2(full.right() - side, full.top()), full.max);
        let middle = Rect::from_min_max(
            pos2(left.right() + 16.0, full.top()),
            pos2(right.left() - 16.0, full.bottom()),
        );

        // Left: cover, title and artist.
        let cover =
            Rect::from_min_size(pos2(left.left(), left.center().y - 28.0), Vec2::splat(56.0));
        match (&self.cover, &self.now.track) {
            (Some((_, texture)), Some(_)) => {
                widgets::paint_tile(ui, cover, Tile::Cover(texture), 4)
            }
            (_, Some(_)) => widgets::paint_tile(ui, cover, Tile::Album, 4),
            _ => {}
        }
        let text = Rect::from_min_max(
            pos2(cover.right() + 14.0, cover.top() + 8.0),
            pos2(left.right(), cover.bottom() - 6.0),
        );
        if let Some(t) = &self.now.track {
            widgets::two_line_text(ui, text, &t.name, TEXT, &t.artists);
        } else {
            let status = self.player_status_text();
            ui.painter().text(
                pos2(left.left(), left.center().y),
                egui::Align2::LEFT_CENTER,
                status,
                theme::regular(13.0),
                SUBDUED,
            );
        }

        // Middle: buttons and the seek bar.
        let ready = matches!(self.player_state, PlayerState::Ready);
        let has_track = self.now.track.is_some();
        let enabled = ready && has_track;
        let buttons_y = middle.top() + 20.0;
        let play_rect =
            Rect::from_center_size(pos2(middle.center().x, buttons_y), Vec2::splat(34.0));
        let prev_rect =
            Rect::from_center_size(pos2(play_rect.left() - 32.0, buttons_y), Vec2::splat(28.0));
        let next_rect =
            Rect::from_center_size(pos2(play_rect.right() + 32.0, buttons_y), Vec2::splat(28.0));
        let mut clicked = |rect: Rect, f: &mut dyn FnMut(&mut Ui) -> egui::Response| {
            widgets::place(ui, egui::UiBuilder::new().max_rect(rect), |ui| {
                f(ui).clicked()
            })
        };
        if clicked(prev_rect, &mut |ui| {
            widgets::icon_button(ui, Icon::Previous, 16.0, enabled).on_hover_text("Previous")
        }) {
            self.previous();
        }
        let playing = self.now.playing;
        if clicked(play_rect, &mut |ui| {
            widgets::play_button(ui, 34.0, playing, TEXT, enabled)
                .on_hover_text("Play/pause (space)")
        }) {
            self.toggle_play();
        }
        if clicked(next_rect, &mut |ui| {
            widgets::icon_button(ui, Icon::Next, 16.0, enabled).on_hover_text("Next")
        }) {
            self.next();
        }

        let duration_ms = self.now.track.as_ref().map(|t| t.duration_ms).unwrap_or(0);
        let shown_ms = self
            .seek_drag
            .map(|v| (v * duration_ms as f32) as u32)
            .unwrap_or_else(|| self.now.position_ms());
        let bar_y = middle.top() + 50.0;
        let time_w = 44.0;
        let bar_rect = Rect::from_min_max(
            pos2(middle.left() + time_w + 8.0, bar_y - 8.0),
            pos2(middle.right() - time_w - 8.0, bar_y + 8.0),
        );
        let time_font = theme::regular(12.0);
        ui.painter().text(
            pos2(bar_rect.left() - 8.0, bar_y),
            egui::Align2::RIGHT_CENTER,
            format_time(shown_ms),
            time_font.clone(),
            SUBDUED,
        );
        ui.painter().text(
            pos2(bar_rect.right() + 8.0, bar_y),
            egui::Align2::LEFT_CENTER,
            format_time(duration_ms),
            time_font,
            SUBDUED,
        );
        let fraction = if duration_ms > 0 {
            shown_ms as f32 / duration_ms as f32
        } else {
            0.0
        };
        let (response, new_value) =
            widgets::place(ui, egui::UiBuilder::new().max_rect(bar_rect), |ui| {
                widgets::thin_slider(ui, fraction, bar_rect.width(), enabled && duration_ms > 0)
            });
        if let Some(v) = new_value {
            self.seek_drag = Some(v);
        }
        // Seek once the user lets go of the bar.
        if let Some(v) = self.seek_drag
            && !response.is_pointer_button_down_on()
            && !response.dragged()
        {
            self.seek_drag = None;
            self.seek((v * duration_ms as f32) as u32);
        }

        // Right: volume.
        let slider_w = 100.0_f32.min(right.width() - 48.0);
        let vol_rect = Rect::from_min_max(
            pos2(right.right() - slider_w, right.center().y - 8.0),
            pos2(right.right(), right.center().y + 8.0),
        );
        let icon_rect = Rect::from_center_size(
            pos2(vol_rect.left() - 20.0, right.center().y),
            Vec2::splat(28.0),
        );
        let icon = match self.config.volume {
            v if v <= 0.0 => Icon::VolumeOff,
            v if v < 0.5 => Icon::VolumeLow,
            _ => Icon::VolumeHigh,
        };
        let mute_clicked = widgets::place(ui, egui::UiBuilder::new().max_rect(icon_rect), |ui| {
            widgets::icon_button(ui, icon, 16.0, true)
                .on_hover_text("Mute")
                .clicked()
        });
        let mut new_volume = None;
        if mute_clicked {
            if self.config.volume > 0.0 {
                self.volume_before_mute = self.config.volume;
                new_volume = Some(0.0);
            } else {
                new_volume = Some(self.volume_before_mute.max(0.1));
            }
        }
        let (response, dragged_volume) =
            widgets::place(ui, egui::UiBuilder::new().max_rect(vol_rect), |ui| {
                widgets::thin_slider(ui, self.config.volume, slider_w, true)
            });
        if let Some(v) = dragged_volume {
            new_volume = Some(v);
        }
        if let Some(v) = new_volume {
            self.config.volume = v;
            if let Some(p) = &self.player {
                p.set_volume(v);
            }
        }
        if mute_clicked || response.drag_stopped() || response.clicked() {
            self.config.save();
        }
    }

    fn player_status_text(&self) -> String {
        match &self.player_state {
            PlayerState::Off => "".into(),
            PlayerState::Connecting => "Connecting the player…".into(),
            PlayerState::BrowserLogin(_) => "Approve playback in your browser".into(),
            PlayerState::Ready => "Double-click a song to play it".into(),
            PlayerState::Failed(_) => "The player is not connected".into(),
        }
    }

    /// Messages about the player that need the user's attention.
    fn player_banner(&mut self, ui: &mut Ui) {
        let mut retry = false;
        let (fill, text, link) = match &self.player_state {
            PlayerState::BrowserLogin(url) => (
                Color32::from_rgb(0x1e, 0x3a, 0x5f),
                "One more step: a browser tab asks you to allow playback. \
                 This is Spotify's own login for the audio player."
                    .to_string(),
                Some(url.clone()),
            ),
            PlayerState::Failed(e) => (
                Color32::from_rgb(0x4a, 0x1c, 0x22),
                format!("Player: {e}"),
                None,
            ),
            _ => return,
        };
        Frame::new()
            .fill(fill)
            .corner_radius(8)
            .inner_margin(Margin::symmetric(16, 12))
            .outer_margin(Margin {
                left: 8,
                right: 8,
                top: 8,
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(text).color(TEXT));
                    if let Some(url) = &link {
                        ui.hyperlink_to("Open it again", url);
                    } else if widgets::pill_button(ui, "Retry", PillStyle::Outline, 13.0, false)
                        .clicked()
                    {
                        retry = true;
                    }
                });
            });
        if retry {
            self.start_player();
        }
    }

    /// The fade at the top of the main card, made once per colour.
    fn header_fade(&mut self, ctx: &egui::Context, color: Color32) -> TextureHandle {
        if let Some((c, texture)) = &self.header_fade
            && *c == color
        {
            return texture.clone();
        }
        let texture = widgets::gradient_texture(ctx, "header", [1, 64], |_, y| {
            widgets::lerp_color(color, CARD, y.powf(0.8))
        });
        self.header_fade = Some((color, texture.clone()));
        texture
    }

    fn main_card(&mut self, ui: &mut Ui) {
        Frame::new().fill(CARD).corner_radius(8).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            self.player_banner(ui);
            if matches!(self.view.kind, ViewKind::Search { .. }) {
                self.search_view(ui);
            } else {
                self.tracks_view(ui);
            }
        });
    }

    fn tracks_view(&mut self, ui: &mut Ui) {
        let ViewKind::Tracks(source) = &self.view.kind else {
            return;
        };
        let source = source.clone();
        let (kind_label, title, color) = match &source {
            TrackSource::LikedSongs => (
                "Playlist",
                "Liked Songs".to_string(),
                theme::HEADER_COLORS[0],
            ),
            TrackSource::Playlist { id, name } => ("Playlist", name.clone(), color_for(id)),
            TrackSource::Album { id, name } => ("Album", name.clone(), color_for(id)),
        };
        let owner = match &source {
            TrackSource::Playlist { id, .. } => self
                .playlists
                .iter()
                .find(|p| p.id == *id)
                .map(|p| p.owner.clone()),
            _ => None,
        };
        let count = self.view.tracks.len();
        let mut details = match &owner {
            Some(o) => format!("{o} · {count} songs"),
            None => format!("{count} songs"),
        };
        if self.view.loading {
            details.push_str(" · loading…");
        }
        let fade = self.header_fade(ui.ctx(), color);
        let is_this_queue = self.playing_source.as_ref() == Some(&source);

        const HEADER: f32 = 236.0;
        const ACTIONS: f32 = 88.0;
        let error_height = if self.view.error.is_some() { 40.0 } else { 0.0 };
        let table_top = HEADER + ACTIONS + error_height;
        let rows_top = table_top + TRACK_HEADER_HEIGHT + 8.0;
        let total = rows_top + count as f32 * TRACK_ROW_HEIGHT + 24.0;

        let mut play_index = None;
        let mut header_play = false;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show_viewport(ui, |ui, viewport| {
                ui.set_min_height(total);
                let origin = ui.max_rect().min;
                let width = ui.max_rect().width();
                let at = |x: f32, y: f32| origin + vec2(x, y);

                // Coloured fade behind the header.
                let fade_rect = Rect::from_min_size(origin, vec2(width, HEADER + ACTIONS));
                egui::Image::new((fade.id(), fade_rect.size()))
                    .corner_radius(CornerRadius {
                        nw: 8,
                        ne: 8,
                        sw: 0,
                        se: 0,
                    })
                    .paint_at(ui, fade_rect);

                // Artwork and title.
                let art = Rect::from_min_size(at(24.0, 52.0), Vec2::splat(160.0));
                let tile = match &source {
                    TrackSource::LikedSongs => Tile::Liked(&self.liked_tile),
                    TrackSource::Playlist { .. } => Tile::Playlist,
                    TrackSource::Album { .. } => Tile::Album,
                };
                widgets::paint_tile(ui, art, tile, 6);
                let text_left = art.right() + 24.0;
                let text_width = (origin.x + width - 24.0 - text_left).max(80.0);
                let p = ui.painter();
                p.text(
                    pos2(text_left, art.top() + 52.0),
                    egui::Align2::LEFT_BOTTOM,
                    kind_label,
                    theme::bold(13.0),
                    TEXT,
                );
                let title_size = fit_font_size(ui, &title, text_width, &[56.0, 44.0, 32.0, 24.0]);
                let title_rect = Rect::from_min_size(
                    pos2(text_left, art.top() + 56.0),
                    vec2(text_width, title_size * 1.3),
                );
                widgets::place(ui, egui::UiBuilder::new().max_rect(title_rect), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&title)
                                .font(theme::bold(title_size))
                                .color(TEXT),
                        )
                        .truncate(),
                    );
                });
                ui.painter().text(
                    pos2(text_left, art.bottom() - 4.0),
                    egui::Align2::LEFT_BOTTOM,
                    &details,
                    theme::regular(14.0),
                    TEXT,
                );

                // Big play button.
                let play_rect = Rect::from_min_size(at(24.0, HEADER + 16.0), Vec2::splat(56.0));
                let playing = is_this_queue && self.now.playing;
                let can_play = matches!(self.player_state, PlayerState::Ready) && count > 0;
                let response =
                    widgets::place(ui, egui::UiBuilder::new().max_rect(play_rect), |ui| {
                        widgets::play_button(ui, 56.0, playing, GREEN, can_play)
                    });
                if response.clicked() {
                    header_play = true;
                }
                if self.view.loading {
                    let spinner = Rect::from_center_size(
                        play_rect.right_center() + vec2(32.0, 0.0),
                        Vec2::splat(22.0),
                    );
                    ui.put(spinner, egui::Spinner::new().color(SUBDUED));
                }

                if let Some(e) = &self.view.error {
                    let rect = Rect::from_min_size(
                        at(24.0, HEADER + ACTIONS),
                        vec2(width - 48.0, error_height),
                    );
                    widgets::place(ui, egui::UiBuilder::new().max_rect(rect), |ui| {
                        ui.label(RichText::new(e).color(ERROR));
                    });
                }

                // The table.
                let header_rect = Rect::from_min_size(
                    at(16.0, table_top),
                    vec2(width - 32.0, TRACK_HEADER_HEIGHT),
                );
                widgets::track_header(ui, header_rect);
                let current = self.now.track.as_ref().map(|t| t.uri.as_str());
                let first = ((viewport.min.y - rows_top) / TRACK_ROW_HEIGHT)
                    .floor()
                    .max(0.0) as usize;
                let last = (((viewport.max.y - rows_top) / TRACK_ROW_HEIGHT)
                    .ceil()
                    .max(0.0) as usize)
                    .min(count);
                for i in first..last {
                    let t = &self.view.tracks[i];
                    let rect = Rect::from_min_size(
                        at(16.0, rows_top + i as f32 * TRACK_ROW_HEIGHT),
                        vec2(width - 32.0, TRACK_ROW_HEIGHT),
                    );
                    let is_current = current == Some(t.uri.as_str());
                    if widgets::track_row(
                        ui,
                        rect,
                        i + 1,
                        t,
                        is_current,
                        is_current && self.now.playing,
                    ) {
                        play_index = Some(i);
                    }
                }
            });

        if header_play {
            if is_this_queue && self.now.track.is_some() {
                self.toggle_play();
            } else {
                self.play_list(self.view.tracks.clone(), 0, Some(source.clone()));
            }
        }
        if let Some(i) = play_index {
            self.play_list(self.view.tracks.clone(), i, Some(source));
        }
    }

    fn search_view(&mut self, ui: &mut Ui) {
        let fade = self.header_fade(ui.ctx(), theme::HEADER_COLORS[7]);
        let ViewKind::Search { query, results } = &self.view.kind else {
            return;
        };
        let mut play = None;
        let mut open = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let top = ui.max_rect().min;
                let width = ui.max_rect().width();
                let fade_rect = Rect::from_min_size(top, vec2(width, 160.0));
                egui::Image::new((fade.id(), fade_rect.size()))
                    .corner_radius(CornerRadius {
                        nw: 8,
                        ne: 8,
                        sw: 0,
                        se: 0,
                    })
                    .paint_at(ui, fade_rect);

                Frame::new()
                    .inner_margin(Margin::symmetric(24, 24))
                    .show(ui, |ui| {
                        ui.set_width(width - 48.0);
                        ui.label(RichText::new("Search").font(theme::bold(13.0)).color(TEXT));
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("\u{201c}{query}\u{201d}"))
                                    .font(theme::bold(40.0))
                                    .color(TEXT),
                            )
                            .truncate(),
                        );
                        ui.add_space(16.0);
                        if self.view.loading {
                            ui.add(egui::Spinner::new().size(24.0).color(GREEN));
                        }
                        if let Some(e) = &self.view.error {
                            ui.label(RichText::new(e).color(ERROR));
                        }
                        let Some(results) = results else { return };

                        section_title(ui, "Songs");
                        if results.tracks.is_empty() {
                            ui.label(RichText::new("No songs found").color(SUBDUED));
                        } else {
                            let current = self.now.track.as_ref().map(|t| t.uri.as_str());
                            let (rect, _) = ui.allocate_exact_size(
                                vec2(ui.available_width(), TRACK_HEADER_HEIGHT),
                                Sense::hover(),
                            );
                            widgets::track_header(ui, rect);
                            ui.add_space(8.0);
                            for (i, t) in results.tracks.iter().enumerate() {
                                let (rect, _) = ui.allocate_exact_size(
                                    vec2(ui.available_width(), TRACK_ROW_HEIGHT),
                                    Sense::hover(),
                                );
                                let is_current = current == Some(t.uri.as_str());
                                if widgets::track_row(
                                    ui,
                                    rect,
                                    i + 1,
                                    t,
                                    is_current,
                                    is_current && self.now.playing,
                                ) {
                                    play = Some(i);
                                }
                            }
                        }

                        ui.add_space(24.0);
                        ui.columns(2, |cols| {
                            section_title(&mut cols[0], "Albums");
                            cols[0].spacing_mut().item_spacing.y = 0.0;
                            if results.albums.is_empty() {
                                cols[0].label(RichText::new("No albums found").color(SUBDUED));
                            }
                            for a in &results.albums {
                                let subtitle = format!("Album · {}", a.artists);
                                if widgets::library_row(
                                    &mut cols[0],
                                    Tile::Album,
                                    &a.name,
                                    &subtitle,
                                    false,
                                    false,
                                )
                                .clicked()
                                {
                                    open = Some(album_view(a));
                                }
                            }
                            section_title(&mut cols[1], "Playlists");
                            cols[1].spacing_mut().item_spacing.y = 0.0;
                            if results.playlists.is_empty() {
                                cols[1].label(RichText::new("No playlists found").color(SUBDUED));
                            }
                            for p in &results.playlists {
                                let subtitle = format!("Playlist · {}", p.owner);
                                if widgets::library_row(
                                    &mut cols[1],
                                    Tile::Playlist,
                                    &p.name,
                                    &subtitle,
                                    false,
                                    false,
                                )
                                .clicked()
                                {
                                    open = Some(ViewKind::Tracks(TrackSource::Playlist {
                                        id: p.id.clone(),
                                        name: p.name.clone(),
                                    }));
                                }
                            }
                        });
                    });
            });
        if let Some(i) = play
            && let ViewKind::Search {
                results: Some(r), ..
            } = &self.view.kind
        {
            let tracks = r.tracks.clone();
            self.play_list(tracks, i, None);
        }
        if let Some(kind) = open {
            self.open(kind);
        }
    }

    /// Short messages at the bottom of the window, like Spotify's blue toasts.
    fn toast(&mut self, ctx: &egui::Context) {
        let Some((text, since)) = &self.notice else {
            return;
        };
        if since.elapsed() > NOTICE_TIME {
            self.notice = None;
            return;
        }
        ctx.request_repaint_after(NOTICE_TIME.saturating_sub(since.elapsed()));
        let text = text.clone();
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -100.0))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let response = Frame::new()
                    .fill(Color32::from_rgb(0x2e, 0x77, 0xd0))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).color(TEXT));
                    })
                    .response;
                if response.interact(Sense::click()).clicked() {
                    self.notice = None;
                }
            });
    }

    /// Fake library for trying out the layout without a Spotify account
    /// (debug builds only: set the LITE_SPOTIFY_DEMO environment variable).
    #[cfg(debug_assertions)]
    fn fill_demo_data(&mut self) {
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
        let source = TrackSource::Playlist {
            id: "demo2".into(),
            name: "Playlist number 2 with a fairly long name".into(),
        };
        self.view.kind = ViewKind::Tracks(source.clone());
        self.view.tracks = (0..120)
            .map(|i| Track {
                uri: format!("spotify:track:demo{i}"),
                name: format!("Song title {i} \u{2014} 東京 Ünïcödé (Remastered 2011)"),
                artists: "Artist One, Artist Two".into(),
                album: "Some Album Name That Is Quite Long".into(),
                duration_ms: 200_000 + i * 1000,
                image_url: None,
                playable: i % 9 != 4,
            })
            .collect();
        self.playing_source = Some(source);
        self.queue = self.view.tracks.clone();
        self.queue_pos = 2;
        self.now = NowPlaying {
            track: Some(self.view.tracks[2].clone()),
            playing: true,
            loading: false,
            position_ms: 65_000,
            since: Some(Instant::now()),
        };
        self.notify("\"Song title 4\" is not available, skipped it.");

        if std::env::var("LITE_SPOTIFY_DEMO").as_deref() == Ok("search") {
            let results = SearchResults {
                tracks: self.view.tracks[..6].to_vec(),
                albums: (1..=5)
                    .map(|i| Album {
                        id: format!("album{i}"),
                        name: format!("Album {i}"),
                        artists: "Some Artist".into(),
                    })
                    .collect(),
                playlists: self.playlists[..5].to_vec(),
            };
            self.view.kind = ViewKind::Search {
                query: "demo search".into(),
                results: Some(results),
            };
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
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
                self.main_screen(ui);
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

/// A numbered step on the setup screen.
fn step(ui: &mut Ui, number: u32, add_contents: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 12.0, GREEN);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            theme::bold(13.0),
            Color32::BLACK,
        );
        ui.add_space(4.0);
        add_contents(ui);
    });
    ui.add_space(6.0);
}

fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).font(theme::bold(22.0)).color(TEXT));
    ui.add_space(8.0);
}

/// The biggest of `sizes` at which `text` fits in `width`.
fn fit_font_size(ui: &Ui, text: &str, width: f32, sizes: &[f32]) -> f32 {
    for &size in sizes {
        let galley = ui
            .painter()
            .layout_no_wrap(text.to_string(), theme::bold(size), TEXT);
        if galley.size().x <= width {
            return size;
        }
    }
    sizes.last().copied().unwrap_or(24.0)
}

/// A steady colour per playlist or album, for the fade behind its header.
fn color_for(id: &str) -> Color32 {
    let hash = id
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    theme::HEADER_COLORS[1 + (hash as usize % 6)]
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
