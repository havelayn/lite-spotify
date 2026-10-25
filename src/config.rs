//! Settings stored on disk, in `%APPDATA%\lite-spotify` on Windows
//! (`~/.config/lite-spotify` on Linux).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Client ID of the user's own Spotify developer app (used for the Web API).
    pub client_id: String,
    /// Web API refresh token, so the user does not have to log in every time.
    pub refresh_token: Option<String>,
    /// Volume from 0.0 to 1.0.
    pub volume: f32,
    /// Name of the accent colour (see `theme::ACCENTS`).
    pub accent: String,
    /// Show the Fun tab (roulette, claw machine and the other song pickers).
    pub fun_enabled: bool,
    /// Play what a Fun tab game picks straight away, instead of asking first.
    pub fun_autoplay: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    /// When the queue runs out (or a single song was played), keep going with
    /// similar songs from Spotify's radio.
    pub autoplay_similar: bool,
    /// The last few searches, newest first.
    pub recent_searches: Vec<String>,
    /// On start, load the song (and queue) that was playing when the app was closed.
    pub resume_session: bool,
    /// Look for a newer version on GitHub when the app starts.
    pub check_updates: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum Repeat {
    #[default]
    Off,
    /// Start the list again after the last song.
    All,
    /// Play the same song again and again.
    One,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            refresh_token: None,
            volume: 0.5,
            accent: "Amber".into(),
            fun_enabled: true,
            fun_autoplay: false,
            shuffle: false,
            repeat: Repeat::Off,
            autoplay_similar: true,
            recent_searches: Vec::new(),
            resume_session: true,
            check_updates: true,
        }
    }
}

/// The folder that holds all of the app's files.
pub fn app_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lite-spotify")
}

/// Where librespot keeps its reusable login (no audio is ever cached).
pub fn player_credentials_dir() -> PathBuf {
    app_dir().join("player")
}

pub fn log_file() -> PathBuf {
    app_dir().join("lite-spotify.log")
}

/// The queue and position at the last exit (see `session.rs`).
pub fn session_file() -> PathBuf {
    app_dir().join("session.json")
}

fn config_file() -> PathBuf {
    app_dir().join("config.json")
}

impl Config {
    pub fn load() -> Self {
        match fs::read_to_string(config_file()) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("config.json is not valid, starting fresh: {e}");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            log::error!("Could not save config: {e}");
        }
    }

    fn try_save(&self) -> std::io::Result<()> {
        write_atomic(
            &config_file(),
            serde_json::to_string_pretty(self)?.as_bytes(),
        )
    }
}

/// Writes to a temporary file first, so a crash never leaves a half-written file.
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(app_dir())?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(tmp, path)
}
