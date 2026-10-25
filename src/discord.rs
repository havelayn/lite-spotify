//! Shows the song in your Discord status ("Listening to …"), through the
//! Discord app running on the same computer. It needs the Application ID of a
//! Discord application the user made; its name is what Discord shows.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use discord_rich_presence::activity::{
    Activity, ActivityType, Assets, StatusDisplayType, Timestamps,
};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

/// What to show. `None` clears the status (paused or nothing playing).
#[derive(Clone, PartialEq)]
pub struct Presence {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_url: Option<String>,
    /// When the song started and will end, in milliseconds since 1970.
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Talks to Discord on a thread of its own, since Discord's connection blocks.
pub struct Discord {
    tx: Sender<Option<Presence>>,
}

impl Discord {
    pub fn start(app_id: String) -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("discord".into())
            .spawn(move || run(app_id, rx))
            .expect("could not start the Discord thread");
        Self { tx }
    }

    pub fn set(&self, presence: Option<Presence>) {
        let _ = self.tx.send(presence);
    }
}

fn run(app_id: String, rx: Receiver<Option<Presence>>) {
    let mut client = DiscordIpcClient::new(&app_id);
    let mut connected = false;
    let mut wanted: Option<Presence> = None;
    let mut dirty = false;
    loop {
        // Wait for a change; every 15 seconds, try again to reach Discord.
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(presence) => {
                wanted = presence;
                dirty = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if !connected {
            if client.connect().is_err() {
                continue; // Discord is not running
            }
            log::info!("Connected to Discord");
            connected = true;
            dirty = true;
        }
        if !dirty {
            continue;
        }
        let result = match &wanted {
            Some(p) => client.set_activity(activity(p)),
            None => client.clear_activity(),
        };
        match result {
            Ok(()) => dirty = false,
            Err(e) => {
                log::info!("Lost the connection to Discord: {e}");
                let _ = client.close();
                connected = false;
            }
        }
    }
    if connected {
        let _ = client.clear_activity();
        let _ = client.close();
    }
}

fn activity(p: &Presence) -> Activity<'_> {
    let mut assets = Assets::new().large_text(&p.album);
    if let Some(url) = &p.cover_url {
        // Discord shows pictures from a web address too.
        assets = assets.large_image(url);
    }
    Activity::new()
        .activity_type(ActivityType::Listening)
        .status_display_type(StatusDisplayType::Details)
        .details(&p.title)
        .state(format!("by {}", p.artist))
        .assets(assets)
        .timestamps(Timestamps::new().start(p.start_ms).end(p.end_ms))
}
