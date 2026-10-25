//! Listening history for the Stats tab. Every song you listen to for at least
//! 30 seconds adds one line to `history.jsonl` in the settings folder. It stays
//! on this computer.

use std::collections::HashMap;
use std::io::Write;

use serde::{Deserialize, Serialize};

use crate::config;

/// A song counts as played after this long, like on Spotify.
pub const MIN_LISTEN_MS: u64 = 30_000;

/// One listen.
#[derive(Serialize, Deserialize, Clone)]
pub struct Play {
    /// When it ended, in seconds since 1970 (UTC).
    pub t: i64,
    pub uri: String,
    pub name: String,
    pub artists: String,
    /// How long it was listened to.
    pub ms: u64,
}

pub fn history_file() -> std::path::PathBuf {
    config::app_dir().join("history.jsonl")
}

pub fn record(play: &Play) {
    let result = (|| {
        std::fs::create_dir_all(config::app_dir())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(history_file())?;
        let mut line = serde_json::to_vec(play).map_err(std::io::Error::other)?;
        line.push(b'\n');
        file.write_all(&line)
    })();
    if let Err(e) = result {
        log::warn!("Could not save the listening history: {e}");
    }
}

/// All listens, oldest first. Damaged lines are skipped.
pub fn load() -> Vec<Play> {
    std::fs::read_to_string(history_file())
        .map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn clear() {
    let _ = std::fs::remove_file(history_file());
}

/// Which listens to count.
#[derive(Clone, Copy, PartialEq)]
pub enum Period {
    Week,
    Month,
    Year,
    AllTime,
}

impl Period {
    pub const ALL: [Period; 4] = [Period::Week, Period::Month, Period::Year, Period::AllTime];

    pub fn name(self) -> &'static str {
        match self {
            Period::Week => "Last 7 days",
            Period::Month => "Last 30 days",
            Period::Year => "Last 12 months",
            Period::AllTime => "All time",
        }
    }

    fn days(self) -> Option<i64> {
        match self {
            Period::Week => Some(7),
            Period::Month => Some(30),
            Period::Year => Some(365),
            Period::AllTime => None,
        }
    }
}

/// A song or artist with how often and how long it was played.
pub struct Ranked {
    pub name: String,
    /// The artist, for songs.
    pub detail: String,
    pub plays: u32,
    pub ms: u64,
}

#[derive(Default)]
pub struct Summary {
    pub total_ms: u64,
    pub plays: u32,
    pub songs: usize,
    pub artists: usize,
    pub top_songs: Vec<Ranked>,
    pub top_artists: Vec<Ranked>,
    /// Listening time per hour of the day (local time).
    pub by_hour: [u64; 24],
    /// Listening time per day of the week, Monday first.
    pub by_weekday: [u64; 7],
}

pub fn summarize(plays: &[Play], period: Period) -> Summary {
    let now = jiff::Timestamp::now().as_second();
    let since = period.days().map(|d| now - d * 24 * 3600);
    let tz = jiff::tz::TimeZone::system();
    let mut summary = Summary::default();
    let mut songs: HashMap<&str, Ranked> = HashMap::new();
    let mut artists: HashMap<&str, Ranked> = HashMap::new();
    for play in plays {
        if since.is_some_and(|s| play.t < s) {
            continue;
        }
        summary.total_ms += play.ms;
        summary.plays += 1;
        let song = songs.entry(play.uri.as_str()).or_insert_with(|| Ranked {
            name: play.name.clone(),
            detail: play.artists.clone(),
            plays: 0,
            ms: 0,
        });
        song.plays += 1;
        song.ms += play.ms;
        for artist in play.artists.split(", ").filter(|a| !a.is_empty()) {
            let entry = artists.entry(artist).or_insert_with(|| Ranked {
                name: artist.to_string(),
                detail: String::new(),
                plays: 0,
                ms: 0,
            });
            entry.plays += 1;
            entry.ms += play.ms;
        }
        if let Ok(when) = jiff::Timestamp::from_second(play.t) {
            let local = when.to_zoned(tz.clone());
            summary.by_hour[local.hour() as usize] += play.ms;
            let weekday = local.weekday().to_monday_zero_offset() as usize;
            summary.by_weekday[weekday] += play.ms;
        }
    }
    summary.songs = songs.len();
    summary.artists = artists.len();
    summary.top_songs = top(songs.into_values().collect());
    summary.top_artists = top(artists.into_values().collect());
    summary
}

/// The ten most played, by plays and then by time.
fn top(mut list: Vec<Ranked>) -> Vec<Ranked> {
    list.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.ms.cmp(&a.ms)));
    list.truncate(10);
    list
}

/// "3 h 25 min", "12 min" or "40 s".
pub fn format_duration(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => format!("{} s", ms / 1000),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h {} min", minutes / 60, minutes % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(t: i64, uri: &str, artists: &str) -> Play {
        Play {
            t,
            uri: uri.into(),
            name: uri.into(),
            artists: artists.into(),
            ms: 60_000,
        }
    }

    #[test]
    fn counts_songs_and_artists() {
        let now = jiff::Timestamp::now().as_second();
        let plays = vec![
            play(now - 100, "a", "X"),
            play(now - 50, "a", "X"),
            play(now - 10, "b", "X, Y"),
            play(now - 40 * 24 * 3600, "c", "Z"),
        ];
        let week = summarize(&plays, Period::Week);
        assert_eq!(week.plays, 3);
        assert_eq!(week.songs, 2);
        assert_eq!(week.artists, 2);
        assert_eq!(week.top_songs[0].name, "a");
        assert_eq!(week.top_artists[0].name, "X");
        assert_eq!(week.top_artists[0].plays, 3);
        assert_eq!(summarize(&plays, Period::AllTime).plays, 4);
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(40_000), "40 s");
        assert_eq!(format_duration(12 * 60_000), "12 min");
        assert_eq!(format_duration(205 * 60_000), "3 h 25 min");
    }
}
