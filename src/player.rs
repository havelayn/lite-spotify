//! Audio playback through librespot.
//!
//! Audio is streamed and played straight away. librespot's audio cache is
//! turned off; the only thing stored on disk is librespot's reusable login.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use futures_util::{StreamExt, stream};
use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::{DeviceType, SessionConfig};
use librespot_core::error::ErrorKind;
use librespot_core::session::Session;
use librespot_core::{Error as SpotError, SpotifyUri};
use librespot_metadata::image::ImageSize;
use librespot_metadata::lyrics::SyncType;
use librespot_metadata::{Lyrics as SpLyrics, Metadata, Playlist as SpPlaylist, Track as SpTrack};
use librespot_playback::audio_backend;
use librespot_playback::config::{AudioFormat, Bitrate, PlayerConfig};
use librespot_playback::mixer::softmixer::SoftMixer;
use librespot_playback::mixer::{Mixer, MixerConfig};
use librespot_playback::player::{Player, PlayerEvent};
use librespot_protocol::autoplay_context_request::AutoplayContextRequest;
use tokio::sync::mpsc;

use crate::api::{Track, join_names};
use crate::auth;
use crate::config;

/// How often Spotify refused to unlock a song since the last song that played.
static KEY_REFUSALS: AtomicU32 = AtomicU32::new(0);

/// Passes log messages on to the log file, and notices when Spotify refuses
/// to hand out the key of a song (librespot only reports that in its log).
/// Spotify does that for some accounts and then no song can play.
pub struct KeyWatch(pub env_logger::Logger);

impl log::Log for KeyWatch {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.0.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if record.level() == log::Level::Error
            && record.target() == "librespot_core::audio_key"
            && record.args().to_string().starts_with("error audio key")
        {
            KEY_REFUSALS.fetch_add(1, Ordering::Relaxed);
        }
        self.0.log(record);
    }

    fn flush(&self) {
        self.0.flush();
    }
}

/// Whether Spotify keeps refusing to unlock songs for this account.
pub fn keys_refused() -> bool {
    KEY_REFUSALS.load(Ordering::Relaxed) >= 2
}

/// Starts counting again (a song played, or the player restarts).
pub fn reset_key_refusals() {
    KEY_REFUSALS.store(0, Ordering::Relaxed);
}

/// What the player tells the UI.
#[derive(Debug)]
pub enum PlayerMsg {
    Connecting,
    /// The user has to approve playback in the browser; holds the login address.
    NeedsBrowserLogin(String),
    Ready,
    Failed(String),
    Loading {
        uri: String,
    },
    Playing {
        uri: String,
        position_ms: u32,
    },
    Paused {
        uri: String,
        position_ms: u32,
    },
    Stopped,
    /// Good moment to tell the player which track comes next.
    PreloadNext,
    EndOfTrack {
        uri: String,
    },
    Unavailable {
        uri: String,
    },
    /// What the Spotify Connect player is doing (controlled from another device).
    Remote(RemoteMsg),
}

/// Events of the Spotify Connect player.
#[derive(Debug)]
pub enum RemoteMsg {
    /// lite-spotify shows up as a Connect device now.
    Ready,
    Failed(String),
    Playing {
        uri: String,
        position_ms: u32,
    },
    Paused {
        uri: String,
        position_ms: u32,
    },
    Stopped,
    /// Volume changed from the other device (0.0 to 1.0).
    Volume(f32),
}

/// The words of a song.
#[derive(Debug, Clone)]
pub struct Lyrics {
    /// Each line has a start time, so the current line can be highlighted.
    pub synced: bool,
    pub lines: Vec<LyricLine>,
    /// Who provided them (shown small underneath, as Spotify asks).
    pub provider: String,
    /// The colours Spotify picked for these lyrics: background, text, and the
    /// line being sung.
    pub colors: Option<[[u8; 3]; 3]>,
}

#[derive(Debug, Clone)]
pub struct LyricLine {
    pub start_ms: u32,
    pub words: String,
}

enum Cmd {
    Load {
        uri: String,
        position_ms: u32,
        play: bool,
    },
    Preload(String),
    /// Play `length_ms` of a track from `position_ms`, then pause (Guess the Song).
    Snippet {
        uri: String,
        position_ms: u32,
        length_ms: u32,
    },
    Play,
    Pause,
    Seek(u32),
    Stop,
    Volume(f32),
}

/// How the player is set up. Changing these restarts the player.
#[derive(Clone, Debug)]
pub struct PlayerOptions {
    pub volume: f32,
    /// Also show up as a Spotify Connect device.
    pub connect: bool,
    /// Streaming quality in kbit/s: 96, 160 or 320.
    pub quality: u16,
    /// Play all songs at about the same loudness.
    pub normalize: bool,
}

/// Handle the UI uses to control playback. Cheap to clone.
#[derive(Clone)]
pub struct PlayerCtl {
    cmds: mpsc::UnboundedSender<Cmd>,
    session: Arc<Mutex<Option<Session>>>,
    /// The Spotify Connect device, while it runs.
    spirc: Arc<Mutex<Option<Spirc>>>,
}

impl PlayerCtl {
    /// Starts logging in and setting up audio in the background.
    /// With `connect`, lite-spotify also shows up as a Spotify Connect device,
    /// so the Spotify app on a phone can play music here.
    pub fn start(
        rt: &tokio::runtime::Handle,
        http: reqwest::Client,
        options: PlayerOptions,
        notify: impl Fn(PlayerMsg) + Send + Sync + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(None));
        let spirc = Arc::new(Mutex::new(None));
        let notify: Notify = Arc::new(notify);
        let connect_slot = options.connect.then(|| spirc.clone());
        rt.spawn(run(
            rx,
            session.clone(),
            http,
            options,
            connect_slot,
            notify,
        ));
        Self {
            cmds: tx,
            session,
            spirc,
        }
    }

    /// Runs `f` on the Spotify Connect device, if it is running.
    fn remote(&self, f: impl FnOnce(&Spirc) -> Result<(), SpotError>) {
        if let Some(spirc) = self.spirc.lock().unwrap().as_ref()
            && let Err(e) = f(spirc)
        {
            log::warn!("Spotify Connect command failed: {e}");
        }
    }
    pub fn remote_play_pause(&self) {
        self.remote(Spirc::play_pause);
    }
    pub fn remote_pause(&self) {
        self.remote(Spirc::pause);
    }
    pub fn remote_next(&self) {
        self.remote(Spirc::next);
    }
    pub fn remote_previous(&self) {
        self.remote(Spirc::prev);
    }
    pub fn remote_seek(&self, position_ms: u32) {
        self.remote(|s| s.set_position_ms(position_ms));
    }
    pub fn remote_volume(&self, volume: f32) {
        self.remote(|s| s.set_volume(to_mixer_volume(volume)));
    }
    /// Stops the Spotify Connect device (when the app closes or it is turned off).
    pub fn stop_connect(&self) {
        self.remote(Spirc::shutdown);
    }

    /// Looks up a song's details through the player's connection.
    pub async fn track_info(&self, uri: &str) -> Result<Track> {
        let session = self.session()?;
        let uri = SpotifyUri::from_uri(uri)?;
        tracks_from_uris(&session, vec![uri])
            .await
            .into_iter()
            .next()
            .context("Could not look up the song")
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.cmds.send(cmd);
    }

    /// Loads a track and starts playing it at `position_ms`.
    pub fn load(&self, uri: &str, position_ms: u32) {
        self.send(Cmd::Load {
            uri: uri.to_string(),
            position_ms,
            play: true,
        });
    }
    /// Loads a track without playing it, ready to continue at `position_ms`.
    pub fn load_paused(&self, uri: &str, position_ms: u32) {
        self.send(Cmd::Load {
            uri: uri.to_string(),
            position_ms,
            play: false,
        });
    }
    /// Plays a short part of a track and pauses by itself. The pause is timed
    /// in the player, so even a tenth of a second is about right.
    pub fn snippet(&self, uri: &str, position_ms: u32, length_ms: u32) {
        self.send(Cmd::Snippet {
            uri: uri.to_string(),
            position_ms,
            length_ms,
        });
    }
    pub fn preload(&self, uri: &str) {
        self.send(Cmd::Preload(uri.to_string()));
    }
    pub fn play(&self) {
        self.send(Cmd::Play);
    }
    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }
    pub fn seek(&self, position_ms: u32) {
        self.send(Cmd::Seek(position_ms));
    }
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }
    pub fn set_volume(&self, volume: f32) {
        self.send(Cmd::Volume(volume));
    }

    fn session(&self) -> Result<Session> {
        self.session
            .lock()
            .unwrap()
            .clone()
            .context("The player is not connected yet")
    }

    /// Reads a playlist through the player's connection. Used for playlists
    /// that Spotify does not let developer apps read through the Web API.
    pub async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let session = self.session()?;
        let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{playlist_id}"))?;
        let playlist = SpPlaylist::get(&session, &uri).await?;
        let uris: Vec<SpotifyUri> = playlist.tracks().take(1000).cloned().collect();
        Ok(tracks_from_uris(&session, uris).await)
    }

    /// The lyrics of a song, from Spotify's own lyrics service.
    pub async fn lyrics(&self, uri: &str) -> Result<Lyrics> {
        let session = self.session()?;
        let SpotifyUri::Track { id } = SpotifyUri::from_uri(uri)? else {
            return Err(anyhow!("Only songs have lyrics"));
        };
        let lyrics = SpLyrics::get(&session, &id)
            .await
            .map_err(|e| anyhow!("Spotify has no lyrics for this song ({e})"))?;
        let synced = lyrics.lyrics.sync_type == SyncType::LineSynced;
        let lines = lyrics
            .lyrics
            .lines
            .into_iter()
            .map(|l| LyricLine {
                start_ms: l.start_time_ms.parse().unwrap_or(0),
                words: l.words,
            })
            .collect();
        let rgb = |argb: i32| {
            let [_, r, g, b] = argb.to_be_bytes();
            [r, g, b]
        };
        let c = &lyrics.colors;
        let colors =
            (c.background != 0).then(|| [rgb(c.background), rgb(c.text), rgb(c.highlight_text)]);
        Ok(Lyrics {
            synced,
            lines,
            provider: lyrics.lyrics.provider_display_name,
            colors,
        })
    }

    /// The songs on Spotify's Top 50 charts: the global one first, then those
    /// of many countries. Each chart is handed to `on_chart` as it loads.
    /// Charts that can't be read are skipped.
    pub async fn chart_tracks(&self, mut on_chart: impl FnMut(Vec<Track>)) -> Result<()> {
        let session = self.session()?;
        let mut seen = std::collections::HashSet::new();
        for id in CHARTS {
            let Ok(uri) = SpotifyUri::from_uri(&format!("spotify:playlist:{id}")) else {
                continue;
            };
            let playlist = match SpPlaylist::get(&session, &uri).await {
                Ok(p) => p,
                Err(e) => {
                    log::info!("Chart {id} could not be read: {e}");
                    continue;
                }
            };
            let uris: Vec<SpotifyUri> = playlist
                .tracks()
                .filter(|u| seen.insert(u.to_uri()))
                .cloned()
                .collect();
            let tracks = tracks_from_uris(&session, uris).await;
            if !tracks.is_empty() {
                on_chart(tracks);
            }
        }
        if seen.is_empty() {
            return Err(anyhow!("Spotify did not let lite-spotify read its charts"));
        }
        Ok(())
    }

    /// Songs similar to `seed_uri`, from Spotify's own autoplay radio (the same
    /// list the Spotify app plays when an album or playlist ends). Songs in
    /// `skip` are left out.
    pub async fn similar_tracks(&self, seed_uri: &str, skip: Vec<String>) -> Result<Vec<Track>> {
        let session = self.session()?;
        let request = AutoplayContextRequest {
            context_uri: Some(seed_uri.to_string()),
            recent_track_uri: skip.iter().rev().take(20).cloned().collect(),
            ..Default::default()
        };
        let context = session.spclient().get_autoplay_context(&request).await?;
        let uris: Vec<SpotifyUri> = context
            .pages
            .iter()
            .flat_map(|page| page.tracks.iter())
            .map(|t| t.uri())
            .filter(|uri| uri.starts_with("spotify:track:") && !skip.iter().any(|s| s == uri))
            .filter_map(|uri| SpotifyUri::from_uri(uri).ok())
            .take(30)
            .collect();
        if uris.is_empty() {
            return Err(anyhow!("Spotify had no similar songs for this one"));
        }
        Ok(tracks_from_uris(&session, uris).await)
    }
}

/// Spotify's own chart playlists: Top 50 Global, Today's Top Hits, Viral 50
/// Global, then the Top 50 of many countries.
const CHARTS: [&str; 22] = [
    "37i9dQZEVXbMDoHDwVN2tF",
    "37i9dQZF1DXcBWIGoYBM5M",
    "37i9dQZEVXbLiRSasKsNU9",
    "37i9dQZEVXbLRQDuF5jeBp",
    "37i9dQZEVXbLnolsZ8PSNw",
    "37i9dQZEVXbKCF6dqVpDkS",
    "37i9dQZEVXbJiZcmkrIHGU",
    "37i9dQZEVXbIPWwFssbupI",
    "37i9dQZEVXbNFJfN1Vw8d9",
    "37i9dQZEVXbIQnj7RRhdSX",
    "37i9dQZEVXbMXbN3EUUhlg",
    "37i9dQZEVXbO3qyFxbkOE1",
    "37i9dQZEVXbKj23U1GF4IR",
    "37i9dQZEVXbJPcfkRz0wJ0",
    "37i9dQZEVXbLoATJ81JYXz",
    "37i9dQZEVXbKXQ4mDTEBXq",
    "37i9dQZEVXbLZ52XmnySJg",
    "37i9dQZEVXbNBz9cRCSFkY",
    "37i9dQZEVXbMMy2roB9myp",
    "37i9dQZEVXbJNSeeHswcKB",
    "37i9dQZEVXbN6itCcaL3Tt",
    "37i9dQZEVXbObFQZ3JLcXt",
];

/// Looks up song details through the player's connection, a few at a time.
/// Songs that can't be looked up are left out; the order is kept.
async fn tracks_from_uris(session: &Session, uris: Vec<SpotifyUri>) -> Vec<Track> {
    stream::iter(uris)
        .map(|uri| {
            let session = session.clone();
            async move { SpTrack::get(&session, &uri).await.ok() }
        })
        .buffered(8)
        .filter_map(|t| async move { t })
        .map(|t| {
            let cover = t
                .album
                .covers
                .iter()
                .find(|i| i.size == ImageSize::DEFAULT)
                .or(t.album.covers.first())
                .map(|i| format!("https://i.scdn.co/image/{}", i.id.to_base16()));
            let thumb_url = t
                .album
                .covers
                .iter()
                .find(|i| i.size == ImageSize::SMALL)
                .map(|i| format!("https://i.scdn.co/image/{}", i.id.to_base16()));
            Track {
                uri: t.id.to_uri(),
                name: t.name,
                artists: join_names(t.artists.iter().map(|a| a.name.clone())),
                year: t.album.date.year().clamp(0, 9999) as u16,
                album: t.album.name,
                duration_ms: t.duration.max(0) as u32,
                image_url: cover,
                playable: true,
                is_episode: false,
                added_at: String::new(),
                album_id: t.album.id.to_id(),
                thumb_url,
            }
        })
        .collect()
        .await
}

/// Runs the Spotify Connect device until it stops. It has its own connection,
/// player and volume, so it never gets in the way of the normal player: the
/// app pauses one when the other starts.
async fn run_connect(
    slot: Arc<Mutex<Option<Spirc>>>,
    options: PlayerOptions,
    notify: Notify,
) -> Result<()> {
    let volume = options.volume;
    let cache = Cache::new(Some(config::player_credentials_dir()), None, None, None)?;
    let credentials = cache
        .credentials()
        .context("The player has no saved login yet")?;
    let session = Session::new(SessionConfig::default(), Some(cache));
    let mixer: Arc<dyn Mixer> = Arc::new(
        SoftMixer::open(MixerConfig::default())
            .map_err(|e| anyhow!("Could not set up volume control: {e}"))?,
    );
    let backend = audio_backend::find(None).context("No audio output available")?;
    let player = Player::new(
        player_config(&options),
        session.clone(),
        mixer.get_soft_volume(),
        move || backend(None, AudioFormat::default()),
    );
    let mut events = player.get_player_event_channel();
    let events_notify = notify.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            let msg = match event {
                PlayerEvent::Playing {
                    track_id,
                    position_ms,
                    ..
                } => RemoteMsg::Playing {
                    uri: track_id.to_uri(),
                    position_ms,
                },
                PlayerEvent::Paused {
                    track_id,
                    position_ms,
                    ..
                } => RemoteMsg::Paused {
                    uri: track_id.to_uri(),
                    position_ms,
                },
                PlayerEvent::Stopped { .. } => RemoteMsg::Stopped,
                PlayerEvent::VolumeChanged { volume } => {
                    RemoteMsg::Volume(volume as f32 / u16::MAX as f32)
                }
                _ => continue,
            };
            events_notify(PlayerMsg::Remote(msg));
        }
    });
    let connect_config = ConnectConfig {
        name: "lite-spotify".into(),
        device_type: DeviceType::Computer,
        initial_volume: to_mixer_volume(volume),
        ..Default::default()
    };
    let (spirc, task) = Spirc::new(connect_config, session, credentials, player, mixer)
        .await
        .map_err(|e| anyhow!("Could not start Spotify Connect: {e}"))?;
    *slot.lock().unwrap() = Some(spirc);
    notify(PlayerMsg::Remote(RemoteMsg::Ready));
    task.await;
    Ok(())
}

fn player_config(options: &PlayerOptions) -> PlayerConfig {
    PlayerConfig {
        bitrate: match options.quality {
            96 => Bitrate::Bitrate96,
            160 => Bitrate::Bitrate160,
            _ => Bitrate::Bitrate320,
        },
        normalisation: options.normalize,
        ..PlayerConfig::default()
    }
}

/// Converts a 0.0–1.0 slider value into librespot's volume range.
fn to_mixer_volume(volume: f32) -> u16 {
    (volume.clamp(0.0, 1.0) * u16::MAX as f32) as u16
}

type Notify = Arc<dyn Fn(PlayerMsg) + Send + Sync>;

async fn run(
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    shared_session: Arc<Mutex<Option<Session>>>,
    http: reqwest::Client,
    options: PlayerOptions,
    connect_slot: Option<Arc<Mutex<Option<Spirc>>>>,
    notify: Notify,
) {
    let volume = options.volume;
    notify(PlayerMsg::Connecting);
    let session = match connect(&http, &notify).await {
        Ok(s) => s,
        Err(e) => {
            log::error!("Player login failed: {e:#}");
            notify(PlayerMsg::Failed(format!("{e:#}")));
            return;
        }
    };
    *shared_session.lock().unwrap() = Some(session.clone());

    let mixer = match SoftMixer::open(MixerConfig::default()) {
        Ok(m) => m,
        Err(e) => {
            notify(PlayerMsg::Failed(format!(
                "Could not set up volume control: {e}"
            )));
            return;
        }
    };
    mixer.set_volume(to_mixer_volume(volume));

    let Some(backend) = audio_backend::find(None) else {
        notify(PlayerMsg::Failed("No audio output available".into()));
        return;
    };
    let player = Player::new(
        player_config(&options),
        session,
        mixer.get_soft_volume(),
        move || backend(None, AudioFormat::default()),
    );

    // The snippet waiting to start playing: its track and length.
    let snippet: Arc<Mutex<Option<(String, u32)>>> = Arc::new(Mutex::new(None));
    let mut events = player.get_player_event_channel();
    let events_notify = notify.clone();
    let (snippet_events, snippet_player) = (snippet.clone(), player.clone());
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let PlayerEvent::Playing { track_id, .. } = &event {
                let uri = track_id.to_uri();
                let mut pending = snippet_events.lock().unwrap();
                if pending.as_ref().is_some_and(|(u, _)| *u == uri)
                    && let Some((_, length_ms)) = pending.take()
                {
                    let player = snippet_player.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(length_ms as u64))
                            .await;
                        player.pause();
                    });
                }
            }
            if let Some(msg) = translate(event) {
                events_notify(msg);
            }
        }
    });

    notify(PlayerMsg::Ready);

    // Spotify Connect starts once the normal player has logged in (and saved
    // the login it reuses).
    if let Some(slot) = connect_slot {
        let notify = notify.clone();
        tokio::spawn(async move {
            if let Err(e) = run_connect(slot.clone(), options.clone(), notify.clone()).await {
                log::warn!("Spotify Connect stopped: {e:#}");
                notify(PlayerMsg::Remote(RemoteMsg::Failed(format!("{e:#}"))));
            }
            *slot.lock().unwrap() = None;
        });
    }

    while let Some(cmd) = cmds.recv().await {
        match cmd {
            Cmd::Load {
                uri,
                position_ms,
                play,
            } => {
                // The connection can drop (network change, laptop sleep); log in again
                // with the saved login before loading the next track.
                let invalid = shared_session
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_none_or(Session::is_invalid);
                if invalid {
                    log::info!("Player connection was lost, reconnecting");
                    match connect(&http, &notify).await {
                        Ok(s) => {
                            player.set_session(s.clone());
                            *shared_session.lock().unwrap() = Some(s);
                            notify(PlayerMsg::Ready);
                        }
                        Err(e) => {
                            notify(PlayerMsg::Failed(format!("{e:#}")));
                            continue;
                        }
                    }
                }
                match SpotifyUri::from_uri(&uri) {
                    Ok(id) => player.load(id, play, position_ms),
                    Err(_) => notify(PlayerMsg::Unavailable { uri }),
                }
            }
            Cmd::Snippet {
                uri,
                position_ms,
                length_ms,
            } => match SpotifyUri::from_uri(&uri) {
                Ok(id) => {
                    *snippet.lock().unwrap() = Some((uri, length_ms));
                    player.load(id, true, position_ms);
                }
                Err(_) => notify(PlayerMsg::Unavailable { uri }),
            },
            Cmd::Preload(uri) => {
                if let Ok(id) = SpotifyUri::from_uri(&uri) {
                    player.preload(id);
                }
            }
            Cmd::Play => player.play(),
            Cmd::Pause => player.pause(),
            Cmd::Seek(ms) => player.seek(ms),
            Cmd::Stop => player.stop(),
            Cmd::Volume(v) => mixer.set_volume(to_mixer_volume(v)),
        }
    }
}

fn translate(event: PlayerEvent) -> Option<PlayerMsg> {
    Some(match event {
        PlayerEvent::Loading { track_id, .. } => PlayerMsg::Loading {
            uri: track_id.to_uri(),
        },
        PlayerEvent::Playing {
            track_id,
            position_ms,
            ..
        } => PlayerMsg::Playing {
            uri: track_id.to_uri(),
            position_ms,
        },
        PlayerEvent::Paused {
            track_id,
            position_ms,
            ..
        } => PlayerMsg::Paused {
            uri: track_id.to_uri(),
            position_ms,
        },
        PlayerEvent::Stopped { .. } => PlayerMsg::Stopped,
        PlayerEvent::TimeToPreloadNextTrack { .. } => PlayerMsg::PreloadNext,
        PlayerEvent::EndOfTrack { track_id, .. } => PlayerMsg::EndOfTrack {
            uri: track_id.to_uri(),
        },
        PlayerEvent::Unavailable { track_id, .. } => PlayerMsg::Unavailable {
            uri: track_id.to_uri(),
        },
        _ => return None,
    })
}

/// Logs in to Spotify's audio servers. Uses the saved login when there is one,
/// otherwise asks the user to approve it in the browser.
async fn connect(http: &reqwest::Client, notify: &Notify) -> Result<Session> {
    let session_config = SessionConfig::default();
    // Only a credentials folder: no volume file, and no audio cache.
    let cache = Cache::new(Some(config::player_credentials_dir()), None, None, None)?;

    if let Some(credentials) = cache.credentials() {
        let session = Session::new(session_config.clone(), Some(cache.clone()));
        match session.connect(credentials, true).await {
            Ok(()) => return Ok(session),
            // The saved login was refused: fall through to a fresh browser login.
            Err(SpotError {
                kind: ErrorKind::PermissionDenied,
                error,
            }) => {
                log::warn!("Saved player login was refused ({error}), logging in again");
            }
            Err(e) => return Err(anyhow!("Could not connect to Spotify: {e}")),
        }
    }

    let tokens = auth::login(
        http,
        &session_config.client_id,
        auth::PLAYER_REDIRECT_URI,
        auth::PLAYER_SCOPES,
        |url| notify(PlayerMsg::NeedsBrowserLogin(url)),
    )
    .await?;
    notify(PlayerMsg::Connecting);
    let session = Session::new(session_config, Some(cache));
    session
        .connect(Credentials::with_access_token(tokens.access_token), true)
        .await
        .map_err(|e| {
            anyhow!(
                "Spotify refused the player login: {e}. Note that playback needs Spotify Premium."
            )
        })?;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use log::Log;

    use super::*;

    #[test]
    fn notices_refused_keys() {
        let watch = KeyWatch(env_logger::Builder::new().build());
        let refusal = |target: &str| {
            watch.log(
                &log::Record::builder()
                    .level(log::Level::Error)
                    .target(target)
                    .args(format_args!("error audio key 0 1"))
                    .build(),
            );
        };
        reset_key_refusals();
        refusal("librespot_core::audio_key");
        assert!(!keys_refused());
        refusal("something_else");
        assert!(!keys_refused());
        refusal("librespot_core::audio_key");
        assert!(keys_refused());
        reset_key_refusals();
        assert!(!keys_refused());
    }
}
