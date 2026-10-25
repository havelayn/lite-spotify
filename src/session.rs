//! What was playing when the app was closed, so it can carry on from there.
//! Stored in `session.json` next to the config. It holds song names and
//! Spotify links only, never any audio.

use serde::{Deserialize, Serialize};

use crate::api::{Track, TrackSource};
use crate::config;

/// At most this many songs of the queue are kept.
const MAX_SAVED: usize = 1000;

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    pub queue: Vec<Track>,
    pub queue_pos: usize,
    pub position_ms: u32,
    pub source: Option<TrackSource>,
    pub user_queued: usize,
}

impl Session {
    /// The saved session, if there is one with a song in it.
    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(config::session_file()).ok()?;
        let session: Session = serde_json::from_str(&text)
            .inspect_err(|e| log::warn!("session.json is not valid: {e}"))
            .ok()?;
        (session.queue_pos < session.queue.len()).then_some(session)
    }

    /// Saves the queue around `queue_pos` (up to [`MAX_SAVED`] songs).
    pub fn save(
        queue: &[Track],
        queue_pos: usize,
        position_ms: u32,
        source: Option<&TrackSource>,
        user_queued: usize,
    ) {
        if queue_pos >= queue.len() {
            let _ = std::fs::remove_file(config::session_file());
            return;
        }
        let start = queue_pos.saturating_sub(100);
        let end = (start + MAX_SAVED).min(queue.len());
        let session = Session {
            queue: queue[start..end].to_vec(),
            queue_pos: queue_pos - start,
            position_ms,
            source: source.cloned(),
            user_queued,
        };
        let result = serde_json::to_vec(&session)
            .map_err(std::io::Error::other)
            .and_then(|json| config::write_atomic(&config::session_file(), &json));
        if let Err(e) = result {
            log::warn!("Could not save the session: {e}");
        }
    }
}
