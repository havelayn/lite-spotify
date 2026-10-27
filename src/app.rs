//! The app's state and what happens behind the screens: signing in, loading
//! lists, the play queue and talking to the player. The screens are in `app/ui.rs`.

mod now_view;
mod ui;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, TextureHandle};
use rand::seq::{IndexedRandom, SliceRandom};

use crate::api::{self, Playlist, SearchResults, Show, Track, TrackSource, WebApi};
use crate::auth;
use crate::config::{self, Config, Repeat};
use crate::discord::{Discord, Presence};
use crate::fun::{Fun, FunAction};
use crate::media::{self, MediaKey};
use crate::player::{Lyrics, PlayerCtl, PlayerMsg, RemoteMsg};
use crate::session::Session;
use crate::stats;
use crate::theme;
use crate::tray::{Tray, TrayCmd};
use crate::update::{self, Release};
use crate::widgets;

/// How long a message at the bottom of the window stays visible.
const NOTICE_TIME: Duration = Duration::from_secs(6);
/// The sleep timer turns the volume down over this long before it pauses.
const SLEEP_FADE: Duration = Duration::from_secs(20);

/// Messages from background tasks to the UI thread.
enum Msg {
    SignInUrl(String),
    SignedIn(Arc<WebApi>, Option<String>),
    SignInFailed(String),
    /// Spotify refused this account for this Client ID: sign out and say why.
    AccountNotAllowed(String),
    NewRefreshToken(String),
    /// Who is signed in, whether the login may change playlists and Liked
    /// Songs, and whether it may read top and recently played songs.
    Me(api::Me, bool, bool),
    /// The user's top songs and artists.
    Top {
        range: api::TopRange,
        result: Result<(Vec<Track>, Vec<api::Artist>), String>,
    },
    /// A thumbnail finished loading (`None` if it failed).
    Thumb {
        url: String,
        image: Option<egui::ColorImage>,
    },
    /// Spotify's listening history was imported.
    Imported(Result<stats::Imported, String>),
    /// Songs from "recently played" were added to the stats.
    RecentMerged(usize),
    /// The user picked a folder to import.
    ImportPicked(std::path::PathBuf),
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
    /// Similar songs for the queue that was playing when they were asked for.
    Similar {
        queue_id: u64,
        result: Result<Vec<Track>, String>,
    },
    /// A playlist was made (and the songs to show it with).
    PlaylistCreated(Result<Playlist, String>),
    /// A playlist was changed; reload it if it is open.
    PlaylistChanged {
        id: String,
        result: Result<String, String>,
    },
    Cover {
        url: String,
        image: egui::ColorImage,
        /// The cover's main colour, for the "Album" accent.
        color: Option<egui::Color32>,
        /// A blurred copy for the glass look's background.
        backdrop: egui::ColorImage,
    },
    Player(PlayerMsg),
    /// Whether a song is in Liked Songs.
    Liked {
        uri: String,
        liked: bool,
    },
    /// Liking or unliking a song worked or failed.
    LikeDone {
        track: Track,
        liked: bool,
        result: Result<(), String>,
    },
    /// A newer version is out.
    Update(Release),
    Tray(TrayCmd),
    /// Details of the song the Spotify Connect device is playing.
    RemoteTrack(Track),
    Lyrics {
        uri: String,
        result: Result<Lyrics, String>,
    },
    MediaKey(MediaKey),
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
    Stats,
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
    /// Only show songs whose title, artist or album contain this.
    filter: String,
    /// Sort by this column (see `widgets::SortColumn`), ascending if true.
    sort: widgets::Sort,
    /// Indices into `tracks` in the order shown, and what they were worked out for.
    shown: Vec<usize>,
    shown_for: Option<(usize, String, widgets::Sort)>,
}

impl ListView {
    fn new(id: u64, source: TrackSource) -> Self {
        Self {
            id,
            source: Some(source),
            loading: true,
            ..Default::default()
        }
    }

    /// Works out which songs to show, in which order, when anything changed.
    fn update_shown(&mut self) {
        let key = (self.tracks.len(), self.filter.clone(), self.sort);
        if self.shown_for.as_ref() == Some(&key) {
            return;
        }
        let words: Vec<String> = self
            .filter
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.shown = (0..self.tracks.len())
            .filter(|&i| {
                let t = &self.tracks[i];
                let text = format!("{} {} {}", t.name, t.artists, t.album).to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .collect();
        if let Some((column, ascending)) = self.sort {
            let tracks = &self.tracks;
            let text = |i: usize| -> String {
                match column {
                    widgets::SortColumn::Title => tracks[i].name.to_lowercase(),
                    widgets::SortColumn::Artist => tracks[i].artists.to_lowercase(),
                    widgets::SortColumn::Album => tracks[i].album.to_lowercase(),
                    widgets::SortColumn::Time => String::new(),
                }
            };
            // Sorting is stable, so equal songs keep the list's own order.
            if column == widgets::SortColumn::Time {
                self.shown.sort_by_key(|&i| tracks[i].duration_ms);
            } else {
                self.shown.sort_by_cached_key(|&i| text(i));
            }
            if !ascending {
                self.shown.reverse();
            }
        }
        self.shown_for = Some(key);
    }

    /// The songs as shown (filtered and sorted).
    fn shown_tracks(&self) -> Vec<Track> {
        self.shown.iter().map(|&i| self.tracks[i].clone()).collect()
    }
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

    /// Spotify user id, to know which playlists the user owns.
    user_id: String,
    /// Whether the login allows making and changing playlists.
    can_edit_playlists: bool,
    /// Whether the login allows reading top and recently played songs.
    can_read_history: bool,
    /// The top songs, artists and albums on the Search tab.
    top: Option<TopView>,
    /// Small covers for lists (when turned on in Settings).
    thumbs: crate::thumbs::Thumbs,
    /// An import of Spotify's listening history is running.
    importing: bool,
    /// A request for similar songs is on its way.
    similar_loading: bool,
    /// Start playing the similar songs as soon as they arrive (the queue ran out).
    similar_then_play: bool,
    /// Changes whenever a new list starts playing, so late similar songs for an
    /// earlier queue are dropped.
    queue_id: u64,
    /// The dialog on top of the window, if any.
    dialog: Option<Dialog>,
    /// Focus the big search box on the Search tab on the next frame.
    focus_search: bool,
    /// When the search box was last typed in, for searching while typing.
    search_typed: Option<Instant>,

    /// Which songs are in Liked Songs, as far as known.
    liked: HashMap<String, bool>,
    /// Windows' media popup and the keyboard's media keys.
    media: Option<media::Controls>,
    /// What the media popup was last told, to only update it when something changes.
    media_shown: Option<(String, media::State)>,
    media_position_at: Option<Instant>,
    sleep: Option<Sleep>,
    /// The music is being faded out by the sleep timer.
    sleep_fading: bool,
    /// The queue saved at the last exit, loaded once the player is ready.
    saved_session: Option<Session>,
    /// A newer version that can be downloaded, until the user hides the banner.
    update: Option<Release>,

    /// The lyrics view is shown instead of the tab.
    lyrics_open: bool,
    /// The full-window Now Playing screen is shown.
    now_view: bool,
    /// Full-screen karaoke: only the lyrics, big.
    karaoke: bool,
    /// The lyrics of this song (`None` while loading).
    lyrics: Option<(String, Option<Result<Lyrics, String>>)>,
    /// The lyrics line that was last scrolled into view.
    lyrics_scrolled_to: Option<usize>,
    /// Guess the Song is playing a snippet. The player panel keeps showing the
    /// song from before, which comes back (paused) when the quiz ends.
    quiz: Option<QuizSnippet>,

    /// The song being listened to, for the stats: how long so far, and since
    /// when it has been playing.
    listening: Option<(Track, u64)>,
    listening_since: Option<Instant>,
    /// The Stats tab's data, loaded while the tab is open.
    stats: Option<StatsView>,

    /// The icon in the notification area (Windows only).
    tray: Option<Tray>,
    tray_tooltip: String,
    /// In the mini player: the window size to go back to.
    mini: Option<egui::Vec2>,
    /// "Quit" was chosen in the tray menu, so closing really closes.
    quitting: bool,
    /// The main colour of the current cover, for the "Album" accent.
    cover_color: Option<egui::Color32>,
    /// The blurred cover behind everything in the glass look, and the
    /// colourful background used when nothing is playing.
    backdrop: Option<TextureHandle>,
    default_backdrop: Option<TextureHandle>,

    /// Another device is playing music here through Spotify Connect; the
    /// buttons control that.
    remote: bool,
    /// The Spotify Connect device is running (or why it is not).
    remote_status: Option<Result<(), String>>,
    /// The Connect song whose details are being looked up: uri, position, playing.
    remote_pending: Option<(String, u32, bool)>,

    /// The accent colour and light/dark look the window and tray icon were
    /// last given.
    window_look: Option<([u8; 3], bool)>,
    /// The Discord status, while turned on.
    discord: Option<Discord>,
    discord_shown: Option<Presence>,
}

struct TopView {
    range: api::TopRange,
    loading: bool,
    tracks: Vec<Track>,
    artists: Vec<api::Artist>,
    /// The albums of the top songs, most songs first.
    albums: Vec<api::Album>,
    error: Option<String>,
    /// Show all 50 songs instead of the first 10.
    show_all: bool,
}

struct StatsView {
    plays: Vec<stats::Play>,
    period: stats::Period,
    summary: stats::Summary,
}

struct QuizSnippet {
    uri: String,
    length_ms: u32,
    /// When to pause; set once the snippet really starts.
    stop_at: Option<Instant>,
}

/// When the sleep timer pauses the music.
#[derive(Clone, Copy, PartialEq)]
enum Sleep {
    At(Instant),
    EndOfSong,
}

/// Small windows on top of the app.
enum Dialog {
    /// Name a new playlist; `tracks` are added to it right away.
    NewPlaylist {
        name: String,
        tracks: Vec<Track>,
    },
    Rename {
        id: String,
        name: String,
    },
    /// The login is from before playlists and Liked Songs could be changed.
    NeedsLogin,
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
        #[allow(unused_mut)]
        let mut config = Config::load();
        // Screenshots of the light look in demo mode (debug builds only).
        #[cfg(debug_assertions)]
        if std::env::var("LITE_SPOTIFY_DEMO_LIGHT").is_ok() {
            config.look = Some(theme::Look::Light);
        }
        theme::setup(&cc.egui_ctx, &config.accent, config.look());

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
            user_id: String::new(),
            can_edit_playlists: false,
            can_read_history: false,
            top: None,
            thumbs: crate::thumbs::Thumbs::default(),
            importing: false,
            similar_loading: false,
            similar_then_play: false,
            queue_id: 0,
            dialog: None,
            focus_search: false,
            search_typed: None,
            liked: HashMap::new(),
            media: None,
            media_shown: None,
            media_position_at: None,
            sleep: None,
            sleep_fading: false,
            saved_session: None,
            update: None,
            lyrics_open: false,
            now_view: false,
            karaoke: false,
            lyrics: None,
            lyrics_scrolled_to: None,
            quiz: None,
            listening: None,
            listening_since: None,
            stats: None,
            tray: None,
            tray_tooltip: String::new(),
            mini: None,
            quitting: false,
            cover_color: None,
            backdrop: None,
            default_backdrop: None,
            remote: false,
            remote_status: None,
            remote_pending: None,
            discord: None,
            discord_shown: None,
            window_look: None,
        };

        app.thumbs.set_enabled(app.config.thumbnails);

        #[cfg(debug_assertions)]
        if let Ok(which) = std::env::var("LITE_SPOTIFY_DEMO") {
            app.fill_demo_data(&which);
            return app;
        }

        let outbox = app.outbox.clone();
        app.media = media::Controls::new(cc, move |key| outbox.send(Msg::MediaKey(key)));
        let outbox = app.outbox.clone();
        app.tray = Tray::new(move |cmd| outbox.send(Msg::Tray(cmd)));
        app.set_discord(app.config.discord_enabled);
        if app.config.autostart {
            // Point the entry at this version's .exe.
            std::thread::spawn(|| crate::autostart::set(true));
        }
        if app.config.resume_session {
            app.saved_session = Session::load();
        }
        if app.config.check_updates {
            let (http, outbox) = (app.http.clone(), app.outbox.clone());
            app.rt.spawn(async move {
                match update::check(&http).await {
                    Ok(Some(release)) => outbox.send(Msg::Update(release)),
                    Ok(None) => {}
                    Err(e) => log::info!("Could not look for updates: {e:#}"),
                }
            });
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
            let me = match api.me().await {
                Ok(me) => Some(me),
                Err(e) if api::is_status(&e, reqwest::StatusCode::FORBIDDEN) => {
                    log::warn!("Spotify refused this account: {e:#}");
                    outbox.send(Msg::AccountNotAllowed(not_allowed_text(&e)));
                    None
                }
                Err(e) => {
                    log::warn!("Could not read the account: {e:#}");
                    None
                }
            };
            if let Some(me) = me {
                let can_edit = api.can_edit().await;
                let can_history = api.can_read_history().await;
                outbox.send(Msg::Me(me, can_edit, can_history));
                if can_history {
                    // Songs played on other devices count for the stats too.
                    match api.recently_played().await {
                        Ok(recent) => {
                            let plays = recent
                                .into_iter()
                                .filter(|r| !r.track.is_episode)
                                .map(|r| stats::Play {
                                    t: r.played_at,
                                    uri: r.track.uri,
                                    name: r.track.name,
                                    artists: r.track.artists,
                                    album: r.track.album,
                                    ms: r.track.duration_ms as u64,
                                })
                                .collect();
                            let added =
                                tokio::task::spawn_blocking(move || stats::merge_recent(plays))
                                    .await
                                    .unwrap_or(0);
                            outbox.send(Msg::RecentMerged(added));
                        }
                        Err(e) => log::info!("Could not read recently played songs: {e:#}"),
                    }
                }
            }
        });
        self.reload_playlists();
        self.open_list(TrackSource::LikedSongs);
        // Signing in again (for new permissions) keeps the player that is already running.
        if self.player.is_none() {
            self.start_player();
        }
    }

    fn start_player(&mut self) {
        crate::player::reset_key_refusals();
        let outbox = self.outbox.clone();
        self.player_state = PlayerState::Connecting;
        self.remote = false;
        self.remote_status = None;
        self.player = Some(PlayerCtl::start(
            self.rt.handle(),
            self.http.clone(),
            crate::player::PlayerOptions {
                volume: self.config.volume,
                connect: self.config.spotify_connect,
                quality: self.config.quality,
                normalize: self.config.normalize,
            },
            move |msg| outbox.send(Msg::Player(msg)),
        ));
    }

    /// Starts the player again, e.g. after turning Spotify Connect on or off.
    fn restart_player(&mut self) {
        if let Some(p) = &self.player {
            p.stop_connect();
            p.stop();
        }
        self.now.playing = false;
        self.now.since = None;
        self.start_player();
    }

    fn sign_out(&mut self) {
        if let Some(p) = &self.player {
            p.stop_connect();
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
        self.liked.clear();
        self.set_sleep(None);
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
            if source == TrackSource::Popular {
                let error = match &player {
                    Some(player) => player
                        .chart_tracks(|tracks| outbox.send(Msg::TracksPage { id, tracks }))
                        .await
                        .err()
                        .map(|e| format!("{e:#}")),
                    None => Some("The player is not connected yet.".into()),
                };
                outbox.send(Msg::TracksDone { id, error });
                return;
            }
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
        let mut view = ListView::new(id, source.clone());
        // Reloading the same list keeps its filter and sorting.
        let old = if matches!(source, TrackSource::Show { .. }) {
            &self.podcast
        } else {
            &self.library
        };
        if old.source.as_ref() == Some(&source) {
            view.filter = old.filter.clone();
            view.sort = old.sort;
        }
        if matches!(source, TrackSource::Show { .. }) {
            self.podcast = view;
            self.tab = Tab::Podcasts;
        } else {
            self.library = view;
            self.tab = Tab::Library;
        }
        self.load_tracks(source, id);
    }

    /// Searches for what is in the search box and remembers it under "Recent
    /// searches".
    fn start_search(&mut self) {
        self.search_typed = None;
        self.run_search(true);
    }

    /// Searches while the user types: shortly after the last key press, and
    /// without adding it to the recent searches.
    fn live_search(&mut self, ctx: &egui::Context) {
        let Some(typed) = self.search_typed else {
            return;
        };
        let wait = Duration::from_millis(450);
        if typed.elapsed() < wait {
            ctx.request_repaint_after(wait.saturating_sub(typed.elapsed()));
            return;
        }
        self.search_typed = None;
        let query = self.search_input.trim();
        if query.chars().count() < 2 || query == self.search.query {
            return;
        }
        if self.tab != Tab::Search {
            // Typing in the top bar moves on to the Search tab's big box.
            self.focus_search = true;
        }
        self.run_search(false);
    }

    fn run_search(&mut self, remember: bool) {
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
        self.lyrics_open = false;
        if remember {
            let recent = &mut self.config.recent_searches;
            recent.retain(|q| !q.eq_ignore_ascii_case(&query));
            recent.insert(0, query.clone());
            recent.truncate(8);
            self.config.save();
        }
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
            FunAction::SaveAsPlaylist(name, tracks) => {
                if self.check_can_edit() {
                    self.dialog = Some(Dialog::NewPlaylist { name, tracks });
                }
            }
            FunAction::QuizPlay {
                track,
                start_ms,
                length_ms,
            } => self.quiz_play(&track, start_ms, length_ms),
            FunAction::QuizEnd => self.end_quiz(),
        }
    }

    /// Plays a few seconds of `track` for Guess the Song, without showing it.
    fn quiz_play(&mut self, track: &Track, start_ms: u32, length_ms: u32) {
        self.leave_remote();
        let Some(player) = &self.player else { return };
        if !self.player_ready() {
            self.notify("The player is not connected yet. Please wait a moment.");
            return;
        }
        if self.quiz.is_none() {
            // Remember where the current song was; it comes back paused afterwards.
            self.now.position_ms = self.now.position_ms();
            self.now.since = None;
            self.now.playing = false;
            self.now.loading = false;
        }
        player.snippet(&track.uri, start_ms, length_ms);
        self.quiz = Some(QuizSnippet {
            uri: track.uri.clone(),
            length_ms,
            stop_at: None,
        });
    }

    /// Ends Guess the Song and loads the song from before, paused.
    fn end_quiz(&mut self) {
        if self.quiz.take().is_none() {
            return;
        }
        let Some(player) = &self.player else { return };
        match &self.now.track {
            Some(t) => {
                player.load_paused(&t.uri, self.now.position_ms);
                self.now.loading = true;
            }
            None => player.stop(),
        }
    }

    /// Pauses the quiz snippet when its time is up.
    fn tick_quiz(&mut self, ctx: &egui::Context) {
        let Some(stop_at) = self.quiz.as_ref().and_then(|q| q.stop_at) else {
            return;
        };
        let left = stop_at.saturating_duration_since(Instant::now());
        if left.is_zero() {
            if let Some(p) = &self.player {
                p.pause();
            }
            if let Some(q) = &mut self.quiz {
                q.stop_at = None;
            }
        } else {
            ctx.request_repaint_after(left);
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
        self.quiz = None;
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
        let single = tracks.len() == 1;
        self.queue = tracks;
        self.queue_id += 1;
        self.similar_then_play = false;
        if self.play_at(index, true) && single {
            // Just one song: line up similar ones after it.
            self.request_similar(false);
        }
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

    /// Called when drawing a screen went wrong: start that part over, so the
    /// app keeps running.
    fn recover_from_crash(&mut self) {
        log::error!("Recovered from a crash on the {} tab", self.tab_name());
        match self.tab {
            Tab::Fun => {
                self.end_quiz();
                if self.fun.is_some() {
                    self.fun = Some(Box::new(Fun::new()));
                }
            }
            Tab::Stats => self.stats = None,
            Tab::Search => {
                self.top = None;
                self.search = SearchView::default();
            }
            Tab::Library | Tab::Podcasts | Tab::Settings => {}
        }
        self.lyrics_open = false;
        self.dialog = None;
        if self.karaoke {
            self.set_karaoke(&self.outbox.ctx.clone(), false);
        }
        self.tab = Tab::Library;
        self.notify(
            "Something went wrong and that screen was reset. Please send the log file \
             (Settings → Open log file) so it can be fixed.",
        );
    }

    fn tab_name(&self) -> &'static str {
        match self.tab {
            Tab::Library => "Library",
            Tab::Search => "Search",
            Tab::Podcasts => "Podcasts",
            Tab::Fun => "Fun",
            Tab::Stats => "Stats",
            Tab::Settings => "Settings",
        }
    }

    /// Puts a song right after the one that is playing.
    fn play_next(&mut self, track: Track) {
        if self.now.track.is_none() || self.queue.is_empty() || self.remote {
            self.play_list(vec![track], 0, None);
            return;
        }
        let name = track.name.clone();
        self.queue.insert(self.queue_pos + 1, track);
        self.user_queued += 1;
        self.notify(format!("\"{name}\" plays next"));
    }

    /// Plays a song, followed by similar songs from Spotify's radio (even when
    /// "keep playing similar songs" is turned off).
    fn start_radio(&mut self, track: Track) {
        let name = track.name.clone();
        let autoplay = self.config.autoplay_similar;
        self.config.autoplay_similar = true;
        self.play_list(vec![track], 0, None);
        self.config.autoplay_similar = autoplay;
        self.notify(format!("Song radio: songs like \"{name}\" come next"));
    }

    /// Copies the song's link on open.spotify.com, to share it.
    fn copy_link(&mut self, track: &Track) {
        let mut parts = track.uri.splitn(3, ':');
        let (_, kind, id) = (parts.next(), parts.next(), parts.next());
        if let (Some(kind), Some(id)) = (kind, id) {
            self.outbox
                .ctx
                .copy_text(format!("https://open.spotify.com/{kind}/{id}"));
            self.notify("Copied the Spotify link");
        }
    }

    /// Takes a song out of Up next (`index` is its place in the queue).
    fn remove_from_queue(&mut self, index: usize) {
        if index <= self.queue_pos || index >= self.queue.len() {
            return;
        }
        if index - self.queue_pos <= self.user_queued {
            self.user_queued = self.user_queued.saturating_sub(1);
        }
        let removed = self.queue.remove(index);
        if let Some(original) = &mut self.unshuffled
            && let Some(p) = original.iter().position(|t| t.uri == removed.uri)
        {
            original.remove(p);
        }
    }

    /// Empties Up next (the song that is playing keeps playing).
    fn clear_queue(&mut self) {
        self.queue.truncate(self.queue_pos + 1);
        self.user_queued = 0;
        self.unshuffled = None;
        self.queue_id += 1;
        self.notify("Up next is empty");
    }

    /// Asks Spotify's radio for songs like the current one and adds them to the
    /// end of the queue. With `then_play`, the queue has already run out and the
    /// first of them starts playing.
    fn request_similar(&mut self, then_play: bool) {
        if !self.config.autoplay_similar {
            return;
        }
        self.similar_then_play |= then_play;
        if self.similar_loading {
            return;
        }
        let (Some(player), Some(track)) = (self.player.clone(), self.now.track.clone()) else {
            return;
        };
        if track.is_episode {
            return;
        }
        self.similar_loading = true;
        let skip: Vec<String> = self.queue.iter().map(|t| t.uri.clone()).collect();
        let outbox = self.outbox.clone();
        let queue_id = self.queue_id;
        self.rt.spawn(async move {
            let result = player
                .similar_tracks(&track.uri, skip)
                .await
                .map_err(|e| format!("{e:#}"));
            outbox.send(Msg::Similar { queue_id, result });
        });
    }

    fn on_similar(&mut self, queue_id: u64, result: Result<Vec<Track>, String>) {
        self.similar_loading = false;
        if queue_id != self.queue_id {
            return; // a different list has started since
        }
        let then_play = std::mem::take(&mut self.similar_then_play);
        match result {
            Ok(mut tracks) => {
                tracks.retain(|t| !self.queue.iter().any(|q| q.uri == t.uri));
                tracks.shuffle(&mut rand::rng());
                let n = tracks.len();
                if n == 0 {
                    return;
                }
                let at_end = self.queue_pos + 1 >= self.queue.len();
                self.queue.extend(tracks);
                if then_play && at_end {
                    self.play_at(self.queue_pos + 1, true);
                } else {
                    self.notify(format!("Added {n} similar songs to Up next"));
                }
            }
            Err(e) => {
                log::warn!("No similar songs: {e}");
                if then_play {
                    self.notify("Reached the end of the list.");
                    self.stop_at_end();
                }
            }
        }
    }

    // ------------------------------------------------------------ playlists

    /// The playlists the signed-in user made (only those can be changed).
    fn own_playlists(&self) -> Vec<(String, String)> {
        self.playlists
            .iter()
            .filter(|p| !self.user_id.is_empty() && p.owner_id == self.user_id)
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect()
    }

    /// Checks that the login allows changing playlists; asks to sign in again if not.
    fn check_can_edit(&mut self) -> bool {
        if !self.can_edit_playlists {
            self.dialog = Some(Dialog::NeedsLogin);
        }
        self.can_edit_playlists
    }

    fn create_playlist(&mut self, name: String, tracks: Vec<Track>) {
        let Some(api) = self.api.clone() else { return };
        if !self.check_can_edit() {
            return;
        }
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = async {
                let playlist = api.create_playlist(&name).await?;
                let uris: Vec<String> = tracks.iter().map(|t| t.uri.clone()).collect();
                if !uris.is_empty() {
                    api.add_to_playlist(&playlist.id, &uris).await?;
                }
                anyhow::Ok(api::Playlist {
                    total: uris.len() as u32,
                    ..playlist
                })
            }
            .await
            .map_err(|e| playlist_error("make the playlist", &e));
            outbox.send(Msg::PlaylistCreated(result));
        });
    }

    fn add_to_playlist(&mut self, playlist_id: String, tracks: Vec<Track>) {
        let Some(api) = self.api.clone() else { return };
        if !self.check_can_edit() {
            return;
        }
        let name = self
            .playlists
            .iter()
            .find(|p| p.id == playlist_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let uris: Vec<String> = tracks.iter().map(|t| t.uri.clone()).collect();
            let result = api
                .add_to_playlist(&playlist_id, &uris)
                .await
                .map(|()| match tracks.as_slice() {
                    [one] => format!("Added \"{}\" to {name}", one.name),
                    _ => format!("Added {} songs to {name}", tracks.len()),
                })
                .map_err(|e| playlist_error("add to the playlist", &e));
            outbox.send(Msg::PlaylistChanged {
                id: playlist_id,
                result,
            });
        });
    }

    fn remove_from_playlist(&mut self, playlist_id: String, track: Track) {
        let Some(api) = self.api.clone() else { return };
        if !self.check_can_edit() {
            return;
        }
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = api
                .remove_from_playlist(&playlist_id, &track.uri)
                .await
                .map(|()| format!("Removed \"{}\"", track.name))
                .map_err(|e| playlist_error("remove the song", &e));
            outbox.send(Msg::PlaylistChanged {
                id: playlist_id,
                result,
            });
        });
    }

    fn rename_playlist(&mut self, playlist_id: String, name: String) {
        let Some(api) = self.api.clone() else { return };
        if !self.check_can_edit() {
            return;
        }
        if let Some(p) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
            p.name = name.clone();
        }
        if let Some(TrackSource::Playlist { id, name: shown }) = &mut self.library.source
            && *id == playlist_id
        {
            *shown = name.clone();
        }
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = api
                .rename_playlist(&playlist_id, &name)
                .await
                .map(|()| format!("Renamed the playlist to \"{name}\""))
                .map_err(|e| playlist_error("rename the playlist", &e));
            outbox.send(Msg::PlaylistChanged {
                id: playlist_id,
                result,
            });
        });
    }

    /// Plays the first playable track at or after (or before, if `forward` is
    /// false) `index` in the queue.
    fn play_at(&mut self, index: usize, forward: bool) -> bool {
        self.leave_remote();
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
        self.check_liked();
        self.save_session();
        true
    }

    // -------------------------------------------------------- saved session

    fn save_session(&self) {
        if self.now.track.is_none() || self.remote {
            return;
        }
        Session::save(
            &self.queue,
            self.queue_pos,
            self.now.position_ms(),
            self.playing_source.as_ref(),
            self.user_queued,
        );
    }

    /// Loads the song that was playing at the last exit, paused where it was.
    fn restore_session(&mut self) {
        let Some(session) = self.saved_session.take() else {
            return;
        };
        let Some(player) = &self.player else { return };
        if self.now.track.is_some() {
            return; // the user already started something else
        }
        let track = session.queue[session.queue_pos].clone();
        player.load_paused(&track.uri, session.position_ms);
        self.queue = session.queue;
        self.queue_pos = session.queue_pos;
        self.user_queued = session.user_queued;
        self.playing_source = session.source;
        self.unshuffled = None;
        self.queue_id += 1;
        self.now = NowPlaying {
            track: Some(track),
            playing: false,
            loading: true,
            position_ms: session.position_ms,
            since: None,
        };
        self.request_cover();
        self.check_liked();
    }

    // ---------------------------------------------------------- Liked Songs

    /// Asks Spotify whether the current song is in Liked Songs, unless that is known.
    fn check_liked(&mut self) {
        let (Some(api), Some(track)) = (self.api.clone(), self.now.track.as_ref()) else {
            return;
        };
        if self.liked.contains_key(&track.uri) {
            return;
        }
        let uri = track.uri.clone();
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            match api.liked(std::slice::from_ref(&uri)).await {
                Ok(answer) => {
                    if let Some(&liked) = answer.first() {
                        outbox.send(Msg::Liked { uri, liked });
                    }
                }
                Err(e) => log::info!("Could not check Liked Songs: {e:#}"),
            }
        });
    }

    fn is_liked(&self, uri: &str) -> bool {
        self.liked.get(uri).copied().unwrap_or(false)
    }

    /// Adds a song to Liked Songs, or removes it if it is there.
    fn toggle_like(&mut self, track: Track) {
        let Some(api) = self.api.clone() else { return };
        if !self.check_can_edit() {
            return;
        }
        let like = !self.is_liked(&track.uri);
        // Show the change straight away; it is undone if Spotify refuses.
        self.liked.insert(track.uri.clone(), like);
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = api
                .set_liked(std::slice::from_ref(&track.uri), like)
                .await
                .map_err(|e| playlist_error("change Liked Songs", &e));
            outbox.send(Msg::LikeDone {
                track,
                liked: like,
                result,
            });
        });
    }

    fn on_like_done(&mut self, track: Track, liked: bool, result: Result<(), String>) {
        if let Err(e) = result {
            self.liked.insert(track.uri, !liked);
            self.notify(e);
            return;
        }
        // Keep an open Liked Songs list up to date.
        if self.library.source == Some(TrackSource::LikedSongs) {
            self.library.tracks.retain(|t| t.uri != track.uri);
            if liked && !track.is_episode {
                self.library.tracks.insert(0, track.clone());
            }
            self.library.shown_for = None;
        }
        self.notify(if liked {
            format!("Added \"{}\" to Liked Songs", track.name)
        } else {
            format!("Removed \"{}\" from Liked Songs", track.name)
        });
    }

    // --------------------------------------------------------------- lyrics

    /// Loads the lyrics of the current song while the lyrics view is open.
    fn update_lyrics(&mut self) {
        if !self.lyrics_open && !self.now_view {
            return;
        }
        let Some(track) = &self.now.track else {
            self.lyrics = None;
            return;
        };
        if self
            .lyrics
            .as_ref()
            .is_some_and(|(uri, _)| *uri == track.uri)
        {
            return;
        }
        let uri = track.uri.clone();
        self.lyrics_scrolled_to = None;
        if track.is_episode {
            self.lyrics = Some((uri, Some(Err("Podcasts don't have lyrics.".into()))));
            return;
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        self.lyrics = Some((uri.clone(), None));
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = player.lyrics(&uri).await.map_err(|e| format!("{e:#}"));
            if let Err(e) = &result {
                log::info!("No lyrics: {e}");
            }
            outbox.send(Msg::Lyrics { uri, result });
        });
    }

    fn set_karaoke(&mut self, ctx: &egui::Context, on: bool) {
        self.karaoke = on;
        if on {
            self.lyrics_open = true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    // ---------------------------------------------------------------- stats

    /// Keeps count of how long the current song has been playing. Runs before
    /// every frame.
    fn track_listening(&mut self) {
        let current = self.now.track.clone().filter(|_| self.quiz.is_none());
        let changed = match (&self.listening, &current) {
            (Some((t, _)), Some(c)) => t.uri != c.uri,
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.finish_listen();
            self.listening = current.filter(|t| !t.is_episode).map(|t| (t.clone(), 0));
        }
        let now = Instant::now();
        if let (Some(since), Some((_, ms))) = (self.listening_since, &mut self.listening) {
            *ms += now.duration_since(since).as_millis() as u64;
        }
        self.listening_since = (self.now.playing && self.listening.is_some()).then_some(now);
    }

    /// Saves the current listen to the history if it was long enough.
    fn finish_listen(&mut self) {
        if let Some(since) = self.listening_since.take()
            && let Some((_, ms)) = &mut self.listening
        {
            *ms += since.elapsed().as_millis() as u64;
        }
        if let Some((track, ms)) = self.listening.take()
            && ms >= stats::MIN_LISTEN_MS
        {
            stats::record(&stats::Play {
                album: track.album,
                t: jiff::Timestamp::now().as_second(),
                uri: track.uri,
                name: track.name,
                artists: track.artists,
                ms,
            });
        }
    }

    /// Loads the user's top songs and artists for the Search tab.
    fn load_top(&mut self, range: api::TopRange) {
        let Some(api) = self.api.clone() else { return };
        let show_all = self.top.as_ref().is_some_and(|t| t.show_all);
        self.top = Some(TopView {
            range,
            loading: true,
            tracks: Vec::new(),
            artists: Vec::new(),
            albums: Vec::new(),
            error: None,
            show_all,
        });
        let outbox = self.outbox.clone();
        self.rt.spawn(async move {
            let result = async {
                let tracks = api.top_tracks(range).await?;
                let artists = api.top_artists(range).await?;
                anyhow::Ok((tracks, artists))
            }
            .await
            .map_err(|e| format!("{e:#}"));
            outbox.send(Msg::Top { range, result });
        });
    }

    fn on_top(
        &mut self,
        range: api::TopRange,
        result: Result<(Vec<Track>, Vec<api::Artist>), String>,
    ) {
        let Some(top) = &mut self.top else { return };
        if top.range != range {
            return;
        }
        top.loading = false;
        match result {
            Ok((tracks, artists)) => {
                // Albums: those with the most top songs first.
                let mut albums: Vec<(api::Album, usize)> = Vec::new();
                for t in &tracks {
                    if t.album_id.is_empty() {
                        continue;
                    }
                    match albums.iter_mut().find(|(a, _)| a.id == t.album_id) {
                        Some((_, n)) => *n += 1,
                        None => albums.push((
                            api::Album {
                                id: t.album_id.clone(),
                                name: t.album.clone(),
                                artists: t.artists.split(", ").next().unwrap_or("").to_string(),
                                image_url: t.thumb_url.clone(),
                            },
                            1,
                        )),
                    }
                }
                albums.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
                top.albums = albums.into_iter().map(|(a, _)| a).take(12).collect();
                top.tracks = tracks;
                top.artists = artists;
            }
            Err(e) => top.error = Some(e),
        }
    }

    /// Imports Spotify's listening history from these files or folders.
    fn start_import(&mut self, paths: Vec<std::path::PathBuf>) {
        if self.importing || paths.is_empty() {
            return;
        }
        self.importing = true;
        let outbox = self.outbox.clone();
        std::thread::spawn(move || {
            let result = stats::import(&paths).map_err(|e| format!("{e:#}"));
            outbox.send(Msg::Imported(result));
        });
    }

    /// Asks for the folder with Spotify's data download (Windows), then imports it.
    fn pick_import_folder(&mut self) {
        let outbox = self.outbox.clone();
        std::thread::spawn(move || {
            if let Some(folder) = crate::files::pick_folder("Choose the folder from Spotify") {
                outbox.send(Msg::ImportPicked(folder));
            }
        });
    }

    fn show_stats(&mut self, period: stats::Period) {
        let plays = match self.stats.take() {
            Some(view) => view.plays,
            None => stats::load(),
        };
        let summary = stats::summarize(&plays, period);
        self.stats = Some(StatsView {
            plays,
            period,
            summary,
        });
    }

    // ------------------------------------------------ mini player and tray

    /// Switches between the full window and the small always-on-top player.
    fn set_mini(&mut self, ctx: &egui::Context, on: bool) {
        use egui::ViewportCommand as V;
        if on && self.mini.is_none() {
            if self.karaoke {
                self.set_karaoke(ctx, false);
            }
            let size = ctx
                .input(|i| i.viewport().inner_rect.map(|r| r.size()))
                .unwrap_or(egui::vec2(1200.0, 760.0));
            self.mini = Some(size);
            ctx.send_viewport_cmd(V::MinInnerSize(egui::vec2(300.0, 116.0)));
            ctx.send_viewport_cmd(V::InnerSize(egui::vec2(400.0, 116.0)));
            ctx.send_viewport_cmd(V::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        } else if !on && let Some(size) = self.mini.take() {
            ctx.send_viewport_cmd(V::WindowLevel(egui::WindowLevel::Normal));
            ctx.send_viewport_cmd(V::MinInnerSize(egui::vec2(860.0, 540.0)));
            ctx.send_viewport_cmd(V::InnerSize(size.max(egui::vec2(860.0, 540.0))));
        }
    }

    /// With the "Album" accent, uses the cover's colour (or the default accent
    /// when the cover is mostly grey).
    fn apply_album_accent(&self, ctx: &egui::Context) {
        if self.config.accent == theme::ALBUM_ACCENT {
            theme::set_accent_color(ctx, self.cover_color.unwrap_or(theme::ACCENTS[0].1));
        }
    }

    /// Downloads the thumbnails that rows on screen asked for.
    fn load_thumbs(&mut self) {
        let Some(api) = self.api.clone() else { return };
        for url in self.thumbs.take_requests() {
            let (api, outbox) = (api.clone(), self.outbox.clone());
            self.rt.spawn(async move {
                let image = async {
                    let bytes = api.image(&url).await.ok()?;
                    let img = image::load_from_memory(&bytes).ok()?;
                    let img = img.thumbnail(64, 64).to_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    Some(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
                }
                .await;
                outbox.send(Msg::Thumb { url, image });
            });
        }
    }

    fn show_window(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn on_tray(&mut self, ctx: &egui::Context, cmd: TrayCmd) {
        match cmd {
            TrayCmd::Show => Self::show_window(ctx),
            TrayCmd::PlayPause => self.toggle_play(),
            TrayCmd::Next if self.now.track.is_some() => self.next(),
            TrayCmd::Previous if self.now.track.is_some() => self.previous(),
            TrayCmd::MiniPlayer => {
                Self::show_window(ctx);
                self.set_mini(ctx, true);
            }
            TrayCmd::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
    }

    /// With "close to tray" on, closing the window hides it instead; the music
    /// keeps playing and the tray icon brings it back.
    fn handle_close(&mut self, ctx: &egui::Context) {
        let close = ctx.input(|i| i.viewport().close_requested());
        if close && self.config.close_to_tray && self.tray.is_some() && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.save_session();
        }
    }

    /// Keeps the window's title bar (dark or light) and the app icon in the
    /// taskbar, Task Manager and tray in the colours of the app's look.
    fn update_window_look(&mut self, ctx: &egui::Context) {
        let look = (
            theme::accent_rgb(),
            self.config.look() == theme::Look::Light,
        );
        if self.window_look == Some(look) {
            return;
        }
        self.window_look = Some(look);
        self.default_backdrop = Some(ctx.load_texture(
            "default-backdrop",
            theme::default_backdrop(),
            egui::TextureOptions::LINEAR,
        ));
        let system_theme = if look.1 {
            egui::SystemTheme::Light
        } else {
            egui::SystemTheme::Dark
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(system_theme));
        ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(
            egui::IconData {
                rgba: crate::tray::icon_rgba(64, look.0),
                width: 64,
                height: 64,
            },
        ))));
        if let Some(tray) = &self.tray {
            tray.set_color(look.0);
        }
    }

    /// Remembers where the window is and how big, for the next start.
    fn remember_window(&mut self, ctx: &egui::Context) {
        if self.mini.is_some() || self.karaoke {
            return;
        }
        let (outer, inner, minimized, fullscreen, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (
                v.outer_rect,
                v.inner_rect,
                v.minimized == Some(true),
                v.fullscreen == Some(true),
                v.maximized == Some(true),
            )
        });
        if minimized || fullscreen || maximized {
            return;
        }
        if let (Some(outer), Some(inner)) = (outer, inner) {
            self.config.window = Some([outer.min.x, outer.min.y, inner.width(), inner.height()]);
        }
    }

    /// The glass look's background: the blurred cover of the song that is
    /// playing (or soft colours), stretched over the window and dimmed so
    /// text stays easy to read.
    fn paint_backdrop(&self, ctx: &egui::Context) {
        if !theme::is_glass() {
            return;
        }
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(14, 16, 24));
        let texture = self
            .backdrop
            .as_ref()
            .filter(|_| self.now.track.is_some())
            .or(self.default_backdrop.as_ref());
        if let Some(texture) = texture {
            // Cover the window, keeping the picture square.
            let side = rect.width().max(rect.height()) * 1.15;
            let square = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(side));
            painter.image(
                texture.id(),
                square,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        // Darker towards the bottom, like Apple Music's now-playing screen.
        let top = egui::Color32::from_black_alpha(110);
        let bottom = egui::Color32::from_black_alpha(170);
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(rect.left_top(), top);
        mesh.colored_vertex(rect.right_top(), top);
        mesh.colored_vertex(rect.right_bottom(), bottom);
        mesh.colored_vertex(rect.left_bottom(), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    }

    /// Shows the song in the tray icon's tooltip.
    fn update_tray(&mut self) {
        let Some(tray) = &self.tray else { return };
        let text = match &self.now.track {
            Some(t) => format!("{} \u{2014} {}", t.name, t.artists),
            None => "lite-spotify".to_string(),
        };
        if text != self.tray_tooltip {
            tray.set_tooltip(&text);
            self.tray_tooltip = text;
        }
    }

    // ------------------------------------------------------ Spotify Connect

    /// Another device started playing here: pause our own player and show
    /// (and control) what the other device plays.
    fn on_remote(&mut self, msg: RemoteMsg) {
        let (uri, position_ms, playing) = match msg {
            RemoteMsg::Ready => {
                self.remote_status = Some(Ok(()));
                return;
            }
            RemoteMsg::Failed(e) => {
                self.remote_status = Some(Err(e));
                self.remote = false;
                return;
            }
            RemoteMsg::Volume(v) => {
                if self.remote {
                    self.config.volume = v;
                }
                return;
            }
            RemoteMsg::Stopped => {
                if self.remote {
                    self.now.playing = false;
                    self.now.since = None;
                }
                return;
            }
            RemoteMsg::Playing { uri, position_ms } => (uri, position_ms, true),
            RemoteMsg::Paused { uri, position_ms } => (uri, position_ms, false),
        };
        if !self.remote {
            if !playing {
                return; // a pause of an earlier Connect session
            }
            self.save_session();
            self.end_quiz();
            if let Some(p) = &self.player {
                p.pause();
            }
            self.remote = true;
            self.notify("Playing from another device through Spotify Connect.");
        }
        if self.now.track.as_ref().is_some_and(|t| t.uri == uri) {
            self.now.playing = playing;
            self.now.loading = false;
            self.now.position_ms = position_ms;
            self.now.since = playing.then(Instant::now);
            return;
        }
        // A new song: look up its name and cover first.
        let lookup = self.remote_pending.as_ref().is_none_or(|(u, ..)| *u != uri);
        self.remote_pending = Some((uri.clone(), position_ms, playing));
        if lookup && let Some(player) = self.player.clone() {
            let outbox = self.outbox.clone();
            self.rt.spawn(async move {
                match player.track_info(&uri).await {
                    Ok(track) => outbox.send(Msg::RemoteTrack(track)),
                    Err(e) => log::warn!("Could not look up the Connect song: {e:#}"),
                }
            });
        }
    }

    fn on_remote_track(&mut self, track: Track) {
        let Some((uri, position_ms, playing)) = self.remote_pending.take() else {
            return;
        };
        if uri != track.uri || !self.remote {
            self.remote_pending = Some((uri, position_ms, playing));
            return;
        }
        self.now = NowPlaying {
            track: Some(track),
            playing,
            loading: false,
            position_ms,
            since: playing.then(Instant::now),
        };
        self.request_cover();
        self.check_liked();
    }

    /// The user plays something here: pause the Connect device and go back to
    /// our own player.
    fn leave_remote(&mut self) {
        if !self.remote {
            return;
        }
        self.remote = false;
        self.remote_pending = None;
        if let Some(p) = &self.player {
            p.remote_pause();
            p.set_volume(self.config.volume);
        }
    }

    // -------------------------------------------------------------- Discord

    fn set_discord(&mut self, on: bool) {
        let id = self.config.discord_app_id.trim().to_string();
        if on && !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
            self.discord = Some(Discord::start(id));
        } else {
            // Dropping it ends its thread, which clears the status.
            self.discord = None;
        }
        self.discord_shown = None;
    }

    /// Tells Discord about the song when it, or play/pause, changed, or after a seek.
    fn update_discord(&mut self) {
        let Some(discord) = &self.discord else { return };
        let presence = match &self.now.track {
            Some(t) if self.now.playing && self.quiz.is_none() => {
                let now_ms = jiff::Timestamp::now().as_millisecond();
                let start_ms = now_ms - self.now.position_ms() as i64;
                Some(Presence {
                    title: t.name.clone(),
                    artist: t.artists.clone(),
                    album: t.album.clone(),
                    cover_url: t.image_url.clone(),
                    start_ms,
                    end_ms: start_ms + t.duration_ms as i64,
                })
            }
            _ => None,
        };
        // Ignore the start time drifting by less than two seconds.
        let same = match (&presence, &self.discord_shown) {
            (Some(a), Some(b)) => {
                a.title == b.title && a.artist == b.artist && (a.start_ms - b.start_ms).abs() < 2000
            }
            (None, None) => true,
            _ => false,
        };
        if !same {
            discord.set(presence.clone());
            self.discord_shown = presence;
        }
    }

    // ---------------------------------------------------------- sleep timer

    fn set_sleep(&mut self, sleep: Option<Sleep>) {
        if self.sleep_fading {
            // Put the volume back where it was.
            self.sleep_fading = false;
            if let Some(p) = &self.player {
                p.set_volume(self.config.volume);
            }
        }
        self.sleep = sleep;
    }

    /// Fades out and pauses when the sleep timer runs out.
    fn tick_sleep(&mut self, ctx: &egui::Context) {
        let Some(Sleep::At(deadline)) = self.sleep else {
            return;
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            if self.now.playing
                && let Some(p) = &self.player
            {
                p.pause();
            }
            self.set_sleep(None);
            self.notify("Sleep timer: the music is paused. Good night!");
        } else if left <= SLEEP_FADE && self.now.playing {
            self.sleep_fading = true;
            let fraction = left.as_secs_f32() / SLEEP_FADE.as_secs_f32();
            if let Some(p) = &self.player {
                p.set_volume(self.config.volume * fraction);
            }
            ctx.request_repaint_after(Duration::from_millis(250));
        } else {
            ctx.request_repaint_after(Duration::from_secs(1).min(left));
        }
    }

    // ---------------------------------------------------------- media keys

    fn on_media_key(&mut self, key: MediaKey) {
        match key {
            MediaKey::Play if !self.now.playing => self.toggle_play(),
            MediaKey::Pause | MediaKey::Stop if self.now.playing => self.toggle_play(),
            MediaKey::Next if self.now.track.is_some() => self.next(),
            MediaKey::Previous if self.now.track.is_some() => self.previous(),
            MediaKey::SeekTo(ms) if self.now.track.is_some() => self.seek(ms),
            _ => {}
        }
    }

    /// Tells Windows' media popup what is playing, when that changed.
    fn update_media_popup(&mut self) {
        let Some(media) = &mut self.media else { return };
        let Some(track) = &self.now.track else {
            if self.media_shown.take().is_some() {
                media.set_song(None);
                media.set_state(media::State::Stopped, 0);
            }
            return;
        };
        let state = if self.now.playing {
            media::State::Playing
        } else if self.now.position_ms() > 0 || self.now.loading {
            media::State::Paused
        } else {
            media::State::Stopped
        };
        let song_changed = self
            .media_shown
            .as_ref()
            .is_none_or(|(uri, _)| *uri != track.uri);
        if song_changed {
            media.set_song(Some(media::SongInfo {
                title: &track.name,
                artist: &track.artists,
                album: &track.album,
                cover_url: track.image_url.as_deref(),
                duration_ms: track.duration_ms,
            }));
        }
        let state_changed = self.media_shown.as_ref().is_none_or(|(_, s)| *s != state);
        // The position is sent now and then; Windows does not move it along itself.
        let position_old = self
            .media_position_at
            .is_none_or(|t| t.elapsed() > Duration::from_secs(5));
        if song_changed || state_changed || position_old {
            media.set_state(state, self.now.position_ms());
            self.media_position_at = Some(Instant::now());
        }
        self.media_shown = Some((track.uri.clone(), state));
    }

    /// Jumps to a song further down the queue (from the Up next list).
    fn jump_to(&mut self, index: usize) {
        self.quiz = None;
        self.user_queued = self
            .user_queued
            .saturating_sub(index.saturating_sub(self.queue_pos));
        self.play_at(index, true);
    }

    fn next(&mut self) {
        if self.remote {
            if let Some(p) = &self.player {
                p.remote_next();
            }
            return;
        }
        self.quiz = None;
        self.user_queued = self.user_queued.saturating_sub(1);
        if self.play_at(self.queue_pos + 1, true) {
            return;
        }
        if self.config.repeat == Repeat::All && self.play_at(0, true) {
            return;
        }
        if self.config.autoplay_similar && self.now.track.as_ref().is_some_and(|t| !t.is_episode) {
            // Keep the music going with similar songs once they arrive.
            self.request_similar(true);
            if self.similar_loading {
                return;
            }
        }
        self.stop_at_end();
    }

    /// The queue ran out: stop and rewind the current song.
    fn stop_at_end(&mut self) {
        if let Some(p) = &self.player {
            p.stop();
        }
        self.now.playing = false;
        self.now.loading = false;
        self.now.position_ms = 0;
        self.now.since = None;
        self.save_session();
    }

    fn previous(&mut self) {
        if self.remote {
            if let Some(p) = &self.player {
                p.remote_previous();
            }
            return;
        }
        self.end_quiz();
        if self.now.position_ms() > 3000 || self.queue_pos == 0 {
            self.seek(0);
        } else {
            self.play_at(self.queue_pos - 1, false);
        }
    }

    fn toggle_play(&mut self) {
        if self.remote {
            if let Some(p) = &self.player {
                p.remote_play_pause();
            }
            return;
        }
        self.end_quiz();
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
        if self.remote {
            if let Some(p) = &self.player {
                p.remote_seek(position_ms);
            }
            return;
        }
        self.end_quiz();
        if let Some(p) = &self.player {
            let duration = self.now.track.as_ref().map_or(0, |t| t.duration_ms);
            let position_ms = position_ms.min(duration.saturating_sub(500));
            p.seek(position_ms);
            self.now.position_ms = position_ms;
            if self.now.since.is_some() {
                self.now.since = Some(Instant::now());
            }
            self.media_position_at = None;
        }
    }

    /// Jumps `seconds` forwards (or backwards when negative) in the current song.
    fn seek_by(&mut self, seconds: i32) {
        if self.now.track.is_none() {
            return;
        }
        let target = self.now.position_ms() as i64 + seconds as i64 * 1000;
        self.seek(target.max(0) as u32);
    }

    fn set_volume(&mut self, volume: f32) {
        self.config.volume = volume;
        if let Some(p) = &self.player {
            if self.remote {
                p.remote_volume(volume);
            } else {
                p.set_volume(volume);
            }
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
            let color = theme::cover_color(img.as_raw());
            let backdrop = theme::backdrop(&img);
            outbox.send(Msg::Cover {
                url,
                image,
                color,
                backdrop,
            });
        });
    }

    fn on_player_msg(&mut self, msg: PlayerMsg) {
        let msg = match msg {
            PlayerMsg::Remote(remote) => return self.on_remote(remote),
            other => other,
        };
        if let Some(quiz) = &mut self.quiz {
            match &msg {
                PlayerMsg::Playing { uri, .. } if *uri == quiz.uri => {
                    if quiz.stop_at.is_none() {
                        // The player pauses by itself; this is only a safety net.
                        quiz.stop_at = Some(
                            Instant::now() + Duration::from_millis(quiz.length_ms as u64 + 400),
                        );
                        if let Some(fun) = &mut self.fun {
                            fun.quiz_started();
                        }
                    }
                    return;
                }
                PlayerMsg::Unavailable { uri } if *uri == quiz.uri => {
                    if let Some(fun) = &mut self.fun {
                        fun.quiz_unavailable();
                    }
                    return;
                }
                // Nothing to preload while the quiz plays.
                PlayerMsg::PreloadNext => return,
                _ => {}
            }
        }
        let current = self.now.track.as_ref().map(|t| t.uri.clone());
        let is_current = |uri: &str| current.as_deref() == Some(uri);
        match msg {
            PlayerMsg::Connecting => self.player_state = PlayerState::Connecting,
            PlayerMsg::NeedsBrowserLogin(url) => self.player_state = PlayerState::BrowserLogin(url),
            PlayerMsg::Ready => {
                self.player_state = PlayerState::Ready;
                self.restore_session();
            }
            PlayerMsg::Failed(e) => {
                self.player_state = PlayerState::Failed(e);
                self.now.loading = false;
            }
            PlayerMsg::Loading { uri } if is_current(&uri) => self.now.loading = true,
            PlayerMsg::Playing { uri, position_ms } if is_current(&uri) => {
                crate::player::reset_key_refusals();
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
                self.save_session();
            }
            PlayerMsg::Stopped => {}
            PlayerMsg::PreloadNext => {
                match (&self.player, self.upcoming()) {
                    (Some(p), Some(t)) => p.preload(&t.uri),
                    // The queue is about to run out: fetch similar songs in time.
                    (Some(_), None) => self.request_similar(false),
                    _ => {}
                }
            }
            PlayerMsg::EndOfTrack { uri } if is_current(&uri) => {
                // Counts each time round when a song repeats.
                self.finish_listen();
                if self.sleep == Some(Sleep::EndOfSong) {
                    self.set_sleep(None);
                    self.stop_at_end();
                    self.notify("Sleep timer: the song has ended. Good night!");
                } else if self.config.repeat == Repeat::One {
                    self.play_at(self.queue_pos, true);
                } else {
                    self.next();
                }
            }
            PlayerMsg::Unavailable { uri } if is_current(&uri) => {
                self.skipped_in_a_row += 1;
                if crate::player::keys_refused() {
                    // Skipping on would fail for every song.
                    self.now.loading = false;
                    self.now.playing = false;
                    self.player_state = PlayerState::Failed(KEYS_REFUSED.into());
                } else if self.skipped_in_a_row > 5 {
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
                Msg::AccountNotAllowed(e) => {
                    self.sign_out();
                    self.screen = Screen::SignIn {
                        url: None,
                        busy: false,
                        error: Some(e),
                    };
                }
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
                Msg::Me(me, can_edit, can_history) => {
                    self.can_read_history = can_history;
                    self.user_name = me.name;
                    self.user_id = me.id;
                    self.can_edit_playlists = can_edit;
                }
                Msg::Similar { queue_id, result } => self.on_similar(queue_id, result),
                Msg::PlaylistCreated(result) => match result {
                    Ok(playlist) => {
                        self.notify(format!("Made the playlist \"{}\"", playlist.name));
                        let source = TrackSource::Playlist {
                            id: playlist.id.clone(),
                            name: playlist.name.clone(),
                        };
                        self.playlists.insert(0, playlist);
                        self.open_list(source);
                    }
                    Err(e) => self.notify(e),
                },
                Msg::PlaylistChanged { id, result } => match result {
                    Ok(message) => {
                        self.notify(message);
                        let open = matches!(&self.library.source,
                            Some(TrackSource::Playlist { id: open, .. }) if *open == id);
                        if open && let Some(source) = self.library.source.clone() {
                            let tab = self.tab;
                            self.open_list(source);
                            self.tab = tab;
                        }
                        self.reload_playlists();
                    }
                    Err(e) => self.notify(e),
                },
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
                    let liked_songs = (id == self.library.id
                        && self.library.source == Some(TrackSource::LikedSongs))
                        || self.fun.as_ref().is_some_and(|f| {
                            f.pool_id() == id && *f.pool_source() == TrackSource::LikedSongs
                        });
                    if liked_songs {
                        for t in &tracks {
                            self.liked.insert(t.uri.clone(), true);
                        }
                    }
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
                Msg::Cover {
                    url,
                    image,
                    color,
                    backdrop,
                } => {
                    let wanted = self.now.track.as_ref().and_then(|t| t.image_url.as_ref());
                    if wanted == Some(&url) {
                        self.cover_color = color;
                        self.apply_album_accent(ctx);
                        self.backdrop = Some(ctx.load_texture(
                            "backdrop",
                            backdrop,
                            egui::TextureOptions::LINEAR,
                        ));
                        let texture =
                            ctx.load_texture("cover", image, egui::TextureOptions::LINEAR);
                        self.cover = Some((url, texture));
                    }
                    self.cover_requested = None;
                }
                Msg::Player(msg) => self.on_player_msg(msg),
                Msg::Liked { uri, liked } => {
                    self.liked.insert(uri, liked);
                }
                Msg::LikeDone {
                    track,
                    liked,
                    result,
                } => self.on_like_done(track, liked, result),
                Msg::Update(release) => self.update = Some(release),
                Msg::Lyrics { uri, result } => {
                    if let Some((wanted, pending)) = &mut self.lyrics
                        && *wanted == uri
                    {
                        *pending = Some(result);
                    }
                }
                Msg::MediaKey(key) => self.on_media_key(key),
                Msg::Top { range, result } => self.on_top(range, result),
                Msg::Thumb { url, image } => self.thumbs.loaded(ctx, url, image),
                Msg::ImportPicked(folder) => self.start_import(vec![folder]),
                Msg::Imported(result) => {
                    self.importing = false;
                    match result {
                        Ok(done) => {
                            self.notify(format!(
                                "Imported {} plays ({}) from {} file(s).",
                                stats::format_count(done.plays as u64),
                                stats::format_duration(done.ms),
                                done.files
                            ));
                            // Show the new numbers.
                            if let Some(view) = self.stats.take() {
                                self.show_stats(view.period);
                            }
                        }
                        Err(e) => self.notify(e),
                    }
                }
                Msg::RecentMerged(added) => {
                    if added > 0 {
                        log::info!("Added {added} recently played songs to the stats");
                        if let Some(view) = self.stats.take() {
                            self.show_stats(view.period);
                        }
                    }
                }
                Msg::Tray(cmd) => self.on_tray(ctx, cmd),
                Msg::RemoteTrack(track) => self.on_remote_track(track),
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
        self.user_id = "demo".into();
        self.can_edit_playlists = true;
        self.config.recent_searches = vec![
            "daft punk".into(),
            "lofi beats".into(),
            "taylor swift".into(),
        ];
        self.player_state = PlayerState::Ready;
        self.playlists = (1..=24)
            .map(|i| Playlist {
                id: format!("demo{i}"),
                name: format!("Playlist number {i} with a fairly long name"),
                owner: "Demo".into(),
                owner_id: "demo".into(),
                total: 20,
                image_url: None,
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
                album_id: String::new(),
                thumb_url: None,
            })
            .collect();
        let source = TrackSource::Playlist {
            id: "demo2".into(),
            name: "Playlist number 2 with a fairly long name".into(),
        };
        self.library = ListView {
            tracks: tracks.clone(),
            loading: false,
            ..ListView::new(1, source.clone())
        };
        for t in tracks.iter().step_by(3) {
            self.liked.insert(t.uri.clone(), true);
        }
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
                image_url: None,
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
                                image_url: None,
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
                            album_id: String::new(),
                            thumb_url: None,
                        })
                        .collect(),
                    loading: false,
                    ..ListView::new(
                        6,
                        TrackSource::Show {
                            id: "show1".into(),
                            name: "Podcast number 1".into(),
                        },
                    )
                };
            }
            "settings" => self.tab = Tab::Settings,
            "mini" => self.mini = Some(egui::vec2(1200.0, 760.0)),
            "stats" => self.tab = Tab::Stats,
            "lyrics" | "karaoke" | "now" => {
                let lines = [
                    "First line of the song",
                    "Here comes the second one",
                    "",
                    "This line is being sung right now",
                    "And this one comes next",
                    "Then a longer line that goes on for quite a while",
                    "Last line",
                ];
                self.lyrics_open = which == "lyrics";
                self.now_view = which == "now";
                self.karaoke = which == "karaoke";
                self.lyrics = Some((
                    tracks[2].uri.clone(),
                    Some(Ok(Lyrics {
                        synced: true,
                        lines: lines
                            .iter()
                            .enumerate()
                            .map(|(i, w)| crate::player::LyricLine {
                                start_ms: i as u32 * 20_000,
                                words: w.to_string(),
                            })
                            .collect(),
                        provider: "Demo".into(),
                        colors: Some([[0x8d, 0x2a, 0x4a], [0x14, 0x08, 0x10], [0xff, 0xff, 0xff]]),
                    })),
                ));
            }
            "sorted" => {
                self.library.filter = "song 1".into();
                self.library.sort = Some((widgets::SortColumn::Title, false));
                self.sleep = Some(Sleep::At(Instant::now() + Duration::from_secs(1500)));
                self.update = Some(Release {
                    version: "9.9.9".into(),
                    page_url: String::new(),
                    download_url: String::new(),
                });
            }
            "search-empty" => {
                self.tab = Tab::Search;
                self.can_read_history = true;
                let artists = [
                    "Artist One",
                    "The Twos",
                    "Band Three",
                    "Four Seasons",
                    "Fifth Ave",
                ];
                self.top = Some(TopView {
                    range: api::TopRange::HalfYear,
                    loading: false,
                    tracks: tracks[..20].to_vec(),
                    artists: artists
                        .iter()
                        .map(|a| api::Artist {
                            name: a.to_string(),
                            image_url: None,
                        })
                        .collect(),
                    albums: (1..=6)
                        .map(|i| api::Album {
                            id: format!("album{i}"),
                            name: format!("Top album {i}"),
                            artists: artists[i % 5].to_string(),
                            image_url: None,
                        })
                        .collect(),
                    error: None,
                    show_all: false,
                });
            }
            game if game.starts_with("fun") => {
                self.tab = Tab::Fun;
                if let Some(fun) = &mut self.fun {
                    fun.start_pool(TrackSource::LikedSongs, 99);
                    // LITE_SPOTIFY_DEMO_POOL=3 tries the games with a tiny list.
                    let size = std::env::var("LITE_SPOTIFY_DEMO_POOL")
                        .ok()
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(tracks.len());
                    fun.add_to_pool(
                        tracks
                            .into_iter()
                            .filter(|t| t.playable)
                            .take(size)
                            .collect(),
                    );
                    fun.pool_done(None);
                    fun.demo_select(game.trim_start_matches("fun").trim_start_matches(':'));
                }
            }
            _ => {}
        }
        self.notify("\"Song title 4\" is not available, skipped it.");
    }
}

/// Shown when Spotify won't unlock songs for this account.
const KEYS_REFUSED: &str = "Spotify won't let this account play music in third-party players \
     right now (it refuses to unlock the songs). This happens to some accounts, mostly newer \
     ones, and nothing in lite-spotify can change it. Browsing, playlists and stats still work. \
     Click Retry to try again later.";

/// Why Spotify refused the account, and how to fix it. Personal developer
/// apps only work while their owner has Premium, and only for the owner and
/// the accounts added to the app's User Management page.
fn not_allowed_text(e: &anyhow::Error) -> String {
    format!(
        "Spotify did not let this account use this Client ID ({e}).\n\n\
         A Spotify developer app only works when the account that created it has \
         Spotify Premium, and only for that account plus accounts its owner adds by \
         hand. Two ways to fix it:\n\
         • Easiest: create your own developer app with this Spotify account (step 1 \
         in the guide), then click \"Use a different Client ID\" and paste your own Client ID.\n\
         • Or the app's owner opens developer.spotify.com/dashboard → the app → \
         User Management, and adds your name and the email of your Spotify account."
    )
}

/// A readable message for a failed playlist change.
fn playlist_error(what: &str, e: &anyhow::Error) -> String {
    if api::is_status(e, reqwest::StatusCode::FORBIDDEN) {
        format!(
            "Spotify did not allow lite-spotify to {what} ({e}). Spotify limits what personal \
             developer apps may change; you can still do it in the Spotify app."
        )
    } else {
        format!("Could not {what}: {e:#}")
    }
}

fn is_forbidden(e: &anyhow::Error) -> bool {
    e.downcast_ref::<api::ApiError>().is_some_and(|e| {
        e.status == reqwest::StatusCode::FORBIDDEN || e.status == reqwest::StatusCode::NOT_FOUND
    })
}
