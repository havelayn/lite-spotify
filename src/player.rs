//! Audio playback through librespot.
//!
//! Audio is streamed and played straight away. librespot's audio cache is
//! turned off; the only thing stored on disk is librespot's reusable login.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use futures_util::{StreamExt, stream};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::SessionConfig;
use librespot_core::error::ErrorKind;
use librespot_core::session::Session;
use librespot_core::{Error as SpotError, SpotifyUri};
use librespot_metadata::image::ImageSize;
use librespot_metadata::{Metadata, Playlist as SpPlaylist, Track as SpTrack};
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
}

enum Cmd {
    Load {
        uri: String,
        position_ms: u32,
        play: bool,
    },
    Preload(String),
    Play,
    Pause,
    Seek(u32),
    Stop,
    Volume(f32),
}

/// Handle the UI uses to control playback. Cheap to clone.
#[derive(Clone)]
pub struct PlayerCtl {
    cmds: mpsc::UnboundedSender<Cmd>,
    session: Arc<Mutex<Option<Session>>>,
}

impl PlayerCtl {
    /// Starts logging in and setting up audio in the background.
    pub fn start(
        rt: &tokio::runtime::Handle,
        http: reqwest::Client,
        volume: f32,
        notify: impl Fn(PlayerMsg) + Send + Sync + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(None));
        rt.spawn(run(rx, session.clone(), http, volume, Arc::new(notify)));
        Self { cmds: tx, session }
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
            }
        })
        .collect()
        .await
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
    volume: f32,
    notify: Notify,
) {
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
    let player_config = PlayerConfig {
        bitrate: Bitrate::Bitrate320,
        ..PlayerConfig::default()
    };
    let player = Player::new(player_config, session, mixer.get_soft_volume(), move || {
        backend(None, AudioFormat::default())
    });

    let mut events = player.get_player_event_channel();
    let events_notify = notify.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Some(msg) = translate(event) {
                events_notify(msg);
            }
        }
    });

    notify(PlayerMsg::Ready);

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
