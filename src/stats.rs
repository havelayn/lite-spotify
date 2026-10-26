//! Listening history for the Stats tab. It stays on this computer, in the
//! settings folder:
//!
//! - `history.jsonl`: every song played in lite-spotify for at least 30
//!   seconds, plus the "recently played" songs Spotify reports from other
//!   devices (added when the app starts).
//! - `imported.jsonl`: listening history imported from Spotify's own data
//!   download ("Download your data" on spotify.com).

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config;

/// A song counts as played after this long, like on Spotify.
pub const MIN_LISTEN_MS: u64 = 30_000;

/// One listen.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Play {
    /// When it ended, in seconds since 1970 (UTC).
    pub t: i64,
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub album: String,
    /// How long it was listened to.
    pub ms: u64,
}

pub fn history_file() -> PathBuf {
    config::app_dir().join("history.jsonl")
}

fn imported_file() -> PathBuf {
    config::app_dir().join("imported.jsonl")
}

fn append(path: &Path, plays: &[Play]) -> std::io::Result<()> {
    std::fs::create_dir_all(config::app_dir())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut out = Vec::new();
    for play in plays {
        serde_json::to_writer(&mut out, play).map_err(std::io::Error::other)?;
        out.push(b'\n');
    }
    file.write_all(&out)
}

pub fn record(play: &Play) {
    if let Err(e) = append(&history_file(), std::slice::from_ref(play)) {
        log::warn!("Could not save the listening history: {e}");
    }
}

fn read(path: &Path) -> Vec<Play> {
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `play` is probably the same listen as one of `known` (same song,
/// ending within a few minutes of each other).
fn already_known(known: &HashMap<String, Vec<i64>>, play: &Play, slack: i64) -> bool {
    known
        .get(&song_key(play))
        .is_some_and(|times| times.iter().any(|t| (t - play.t).abs() <= slack))
}

fn index(plays: &[Play]) -> HashMap<String, Vec<i64>> {
    let mut known: HashMap<String, Vec<i64>> = HashMap::new();
    for p in plays {
        known.entry(song_key(p)).or_default().push(p.t);
    }
    known
}

/// All listens, oldest first: this app's history and the imported one, with
/// listens that are in both counted once. Damaged lines are skipped.
pub fn load() -> Vec<Play> {
    let mut plays = read(&history_file());
    let known = index(&plays);
    plays.extend(
        read(&imported_file())
            .into_iter()
            .filter(|p| !already_known(&known, p, 180)),
    );
    plays.sort_by_key(|p| p.t);
    plays
}

/// Adds the songs Spotify says were played recently (on any device) that are
/// not in the history yet. Returns how many were added.
pub fn merge_recent(recent: Vec<Play>) -> usize {
    let history = read(&history_file());
    let known = index(&history);
    let new: Vec<Play> = recent
        .into_iter()
        // Spotify gives the start time; the history has end times.
        .filter(|p| !already_known(&known, p, p.ms as i64 / 1000 + 180))
        .collect();
    if !new.is_empty()
        && let Err(e) = append(&history_file(), &new)
    {
        log::warn!("Could not save recently played songs: {e}");
        return 0;
    }
    new.len()
}

pub fn clear() {
    let _ = std::fs::remove_file(history_file());
    let _ = std::fs::remove_file(imported_file());
}

// ---- Importing Spotify's data download ----

/// What an import found.
pub struct Imported {
    pub files: usize,
    pub plays: usize,
    pub ms: u64,
}

/// Imports the listening history from Spotify's data download: the
/// `StreamingHistory_music_*.json` files ("Account data") or the
/// `Streaming_History_Audio_*.json` files ("Extended streaming history").
/// `paths` are those files or folders that hold them. Importing the same
/// files again does not count them twice.
pub fn import(paths: &[PathBuf]) -> Result<Imported> {
    let mut files = Vec::new();
    for path in paths {
        collect_files(path, &mut files, 0);
    }
    if files.is_empty() {
        bail!(
            "No listening history found. Choose the folder you unzipped from Spotify \
             (it has files like StreamingHistory_music_0.json)."
        );
    }
    let mut plays = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("Could not read {}", file.display()))?;
        let entries: Vec<Value> = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a Spotify history file", file.display()))?;
        plays.extend(entries.iter().filter_map(parse_entry));
    }
    let found = plays.len();
    let ms = plays.iter().map(|p| p.ms).sum();

    // Merge with what was imported before, without doubles.
    let mut all = read(&imported_file());
    let mut seen: HashSet<(i64, String)> = all.iter().map(|p| (p.t, song_key(p))).collect();
    all.extend(
        plays
            .into_iter()
            .filter(|p| seen.insert((p.t, song_key(p)))),
    );
    all.sort_by_key(|p| p.t);
    let mut out = Vec::new();
    for play in &all {
        serde_json::to_writer(&mut out, play)?;
        out.push(b'\n');
    }
    config::write_atomic(&imported_file(), &out)?;
    Ok(Imported {
        files: files.len(),
        plays: found,
        ms,
    })
}

fn collect_files(path: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if path.is_dir() {
        if depth > 3 {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                collect_files(&entry.path(), out, depth + 1);
            }
        }
    } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let name = name.to_lowercase();
        let history = name.starts_with("streaminghistory")
            || name.starts_with("streaming_history_audio")
            || name.starts_with("endsong");
        // StreamingHistory_podcast_*.json holds podcasts; only music counts.
        if history && name.ends_with(".json") && !name.contains("podcast") {
            out.push(path.to_path_buf());
        }
    }
}

/// One entry of either kind of Spotify history file.
fn parse_entry(v: &Value) -> Option<Play> {
    let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(ts) = v.get("ts").and_then(Value::as_str) {
        // Extended streaming history. Podcast episodes have no track name.
        let name = text("master_metadata_track_name");
        if name.is_empty() {
            return None;
        }
        let artists = text("master_metadata_album_artist_name");
        let uri = text("spotify_track_uri");
        return Some(Play {
            t: ts.parse::<jiff::Timestamp>().ok()?.as_second(),
            uri: if uri.is_empty() {
                format!("import:{artists}|{name}")
            } else {
                uri
            },
            name,
            artists,
            album: text("master_metadata_album_album_name"),
            ms: v.get("ms_played")?.as_u64()?,
        });
    }
    // Account data: "endTime": "2024-01-31 18:05" in UTC.
    let end = v.get("endTime").and_then(Value::as_str)?;
    let t = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M", end)
        .ok()?
        .to_zoned(jiff::tz::TimeZone::UTC)
        .ok()?
        .timestamp()
        .as_second();
    let name = text("trackName");
    let artists = text("artistName");
    if name.is_empty() {
        return None;
    }
    Some(Play {
        t,
        uri: format!("import:{artists}|{name}"),
        name,
        artists,
        album: String::new(),
        ms: v.get("msPlayed")?.as_u64()?,
    })
}

// ---- Summaries ----

/// Songs are matched by title and first artist, since imported plays from
/// Spotify's "Account data" have no song links.
fn song_key(p: &Play) -> String {
    let artist = p.artists.split(", ").next().unwrap_or("");
    format!("{}\u{1f}{}", p.name.to_lowercase(), artist.to_lowercase())
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

/// A song, artist or album with how often and how long it was played.
pub struct Ranked {
    pub name: String,
    /// The artist, for songs and albums.
    pub detail: String,
    pub plays: u32,
    pub ms: u64,
}

#[derive(Default)]
pub struct Summary {
    pub total_ms: u64,
    /// Listens of at least 30 seconds.
    pub plays: u32,
    pub songs: usize,
    pub artists: usize,
    pub albums: usize,
    pub top_songs: Vec<Ranked>,
    pub top_artists: Vec<Ranked>,
    pub top_albums: Vec<Ranked>,
    /// Listening time per hour of the day (local time).
    pub by_hour: [u64; 24],
    /// Listening time per day of the week, Monday first.
    pub by_weekday: [u64; 7],
    /// Listening time over the period: per day for a week or month, per month
    /// for a year, per year for all time.
    pub timeline: Vec<(String, u64)>,
    /// Days with any listening.
    pub days: usize,
    /// The most days in a row with listening, and the current run.
    pub longest_streak: usize,
    pub current_streak: usize,
    /// The day with the most listening, and how much.
    pub busiest_day: Option<(String, u64)>,
    /// Artists heard for the first time in this period (not for "all time").
    pub new_artists: Option<usize>,
    /// The first listen in the history.
    pub first_listen: Option<String>,
}

pub fn summarize(plays: &[Play], period: Period) -> Summary {
    let tz = jiff::tz::TimeZone::system();
    let now = jiff::Timestamp::now().to_zoned(tz.clone());
    let since = period
        .days()
        .map(|d| now.timestamp().as_second() - d * 24 * 3600);
    let mut summary = Summary {
        first_listen: plays
            .first()
            .and_then(|p| jiff::Timestamp::from_second(p.t).ok())
            .map(|t| t.to_zoned(tz.clone()).strftime("%-d %B %Y").to_string()),
        ..Default::default()
    };
    let mut songs: HashMap<String, Ranked> = HashMap::new();
    let mut artists: HashMap<&str, Ranked> = HashMap::new();
    let mut albums: HashMap<String, Ranked> = HashMap::new();
    let mut per_day: HashMap<jiff::civil::Date, u64> = HashMap::new();
    let mut timeline: HashMap<i64, u64> = HashMap::new();
    let mut artists_before: HashSet<&str> = HashSet::new();

    for play in plays {
        if since.is_some_and(|s| play.t < s) {
            for artist in play.artists.split(", ") {
                artists_before.insert(artist);
            }
            continue;
        }
        let counts = play.ms >= MIN_LISTEN_MS;
        let add = |r: &mut Ranked| {
            r.plays += u32::from(counts);
            r.ms += play.ms;
        };
        summary.total_ms += play.ms;
        summary.plays += u32::from(counts);
        add(songs.entry(song_key(play)).or_insert_with(|| Ranked {
            name: play.name.clone(),
            detail: play.artists.clone(),
            plays: 0,
            ms: 0,
        }));
        for artist in play.artists.split(", ").filter(|a| !a.is_empty()) {
            add(artists.entry(artist).or_insert_with(|| Ranked {
                name: artist.to_string(),
                detail: String::new(),
                plays: 0,
                ms: 0,
            }));
        }
        if !play.album.is_empty() {
            let first_artist = play.artists.split(", ").next().unwrap_or("");
            let key = format!("{}\u{1f}{}", play.album.to_lowercase(), first_artist);
            add(albums.entry(key).or_insert_with(|| Ranked {
                name: play.album.clone(),
                detail: first_artist.to_string(),
                plays: 0,
                ms: 0,
            }));
        }
        let Ok(when) = jiff::Timestamp::from_second(play.t) else {
            continue;
        };
        let local = when.to_zoned(tz.clone());
        summary.by_hour[local.hour() as usize] += play.ms;
        summary.by_weekday[local.weekday().to_monday_zero_offset() as usize] += play.ms;
        *per_day.entry(local.date()).or_default() += play.ms;
        let bucket = match period {
            Period::Week | Period::Month => (now.date() - local.date()).get_days() as i64,
            Period::Year => {
                (now.year() as i64 - local.year() as i64) * 12 + now.month() as i64
                    - local.month() as i64
            }
            Period::AllTime => now.year() as i64 - local.year() as i64,
        };
        *timeline.entry(bucket).or_default() += play.ms;
    }

    summary.songs = songs.len();
    summary.artists = artists.len();
    summary.albums = albums.len();
    if since.is_some() {
        summary.new_artists = Some(
            artists
                .keys()
                .filter(|a| !artists_before.contains(*a))
                .count(),
        );
    }
    summary.top_songs = top(songs.into_values().collect());
    summary.top_artists = top(artists.into_values().collect());
    summary.top_albums = top(albums.into_values().collect());

    // Days, streaks and the busiest day.
    summary.days = per_day.len();
    let mut days: Vec<jiff::civil::Date> = per_day.keys().copied().collect();
    days.sort();
    let mut run = 0;
    let mut previous: Option<jiff::civil::Date> = None;
    for day in &days {
        run = match previous {
            Some(p) if p.tomorrow().ok() == Some(*day) => run + 1,
            _ => 1,
        };
        summary.longest_streak = summary.longest_streak.max(run);
        previous = Some(*day);
    }
    let today = now.date();
    let yesterday = today.yesterday().ok();
    summary.current_streak = match days.last() {
        Some(last) if *last == today || Some(*last) == yesterday => run,
        _ => 0,
    };
    summary.busiest_day = per_day
        .iter()
        .max_by_key(|(_, ms)| **ms)
        .map(|(day, ms)| (day.strftime("%-d %B %Y").to_string(), *ms));

    // The timeline, oldest first.
    let buckets: i64 = match period {
        Period::Week => 7,
        Period::Month => 30,
        Period::Year => 12,
        Period::AllTime => timeline.keys().max().map_or(1, |m| m + 1).min(15),
    };
    summary.timeline = (0..buckets)
        .rev()
        .map(|b| {
            let label = match period {
                Period::Week => today
                    .checked_sub(jiff::Span::new().days(b))
                    .map(|d| d.strftime("%a").to_string())
                    .unwrap_or_default(),
                Period::Month => today
                    .checked_sub(jiff::Span::new().days(b))
                    .map(|d| {
                        if b % 5 == 0 {
                            d.strftime("%-d %b").to_string()
                        } else {
                            String::new()
                        }
                    })
                    .unwrap_or_default(),
                Period::Year => today
                    .first_of_month()
                    .checked_sub(jiff::Span::new().months(b))
                    .map(|d| d.strftime("%b").to_string())
                    .unwrap_or_default(),
                Period::AllTime => (today.year() as i64 - b).to_string(),
            };
            (label, timeline.get(&b).copied().unwrap_or(0))
        })
        .collect();
    summary
}

/// The ten most played, by plays and then by time.
fn top(mut list: Vec<Ranked>) -> Vec<Ranked> {
    list.retain(|r| r.ms > 0);
    list.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.ms.cmp(&a.ms)));
    list.truncate(10);
    list
}

/// "3 h 25 min", "12 min" or "40 s"; days are written as hours ("52 h").
pub fn format_duration(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => format!("{} s", ms / 1000),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h {} min", minutes / 60, minutes % 60),
    }
}

/// "12,345": big numbers with a thousands separator.
pub fn format_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(t: i64, name: &str, artists: &str) -> Play {
        Play {
            t,
            uri: name.into(),
            name: name.into(),
            artists: artists.into(),
            album: "Album".into(),
            ms: 60_000,
        }
    }

    #[test]
    fn counts_songs_and_artists() {
        let now = jiff::Timestamp::now().as_second();
        let plays = vec![
            play(now - 40 * 24 * 3600, "c", "Z"),
            play(now - 100, "a", "X"),
            play(now - 50, "a", "X"),
            play(now - 10, "b", "X, Y"),
        ];
        let week = summarize(&plays, Period::Week);
        assert_eq!(week.plays, 3);
        assert_eq!(week.songs, 2);
        assert_eq!(week.artists, 2);
        assert_eq!(week.top_songs[0].name, "a");
        assert_eq!(week.top_artists[0].name, "X");
        assert_eq!(week.top_artists[0].plays, 3);
        assert_eq!(week.new_artists, Some(2));
        assert_eq!(week.timeline.len(), 7);
        assert_eq!(summarize(&plays, Period::AllTime).plays, 4);
    }

    #[test]
    fn short_listens_count_time_but_not_plays() {
        let now = jiff::Timestamp::now().as_second();
        let mut short = play(now, "a", "X");
        short.ms = 10_000;
        let sum = summarize(&[short], Period::Week);
        assert_eq!(sum.plays, 0);
        assert_eq!(sum.total_ms, 10_000);
    }

    #[test]
    fn reads_both_kinds_of_spotify_history() {
        let extended = serde_json::json!({
            "ts": "2024-03-01T12:00:00Z",
            "ms_played": 200000,
            "master_metadata_track_name": "Song",
            "master_metadata_album_artist_name": "Artist",
            "master_metadata_album_album_name": "Album",
            "spotify_track_uri": "spotify:track:abc"
        });
        let p = parse_entry(&extended).unwrap();
        assert_eq!(
            (p.name.as_str(), p.ms, p.uri.as_str()),
            ("Song", 200000, "spotify:track:abc")
        );
        let account = serde_json::json!({
            "endTime": "2024-03-01 12:00",
            "artistName": "Artist",
            "trackName": "Song",
            "msPlayed": 1234
        });
        let p = parse_entry(&account).unwrap();
        assert_eq!((p.t, p.ms), (1709294400, 1234));
        let podcast = serde_json::json!({ "ts": "2024-03-01T12:00:00Z", "ms_played": 5, "episode_name": "x" });
        assert!(parse_entry(&podcast).is_none());
    }

    #[test]
    fn formats_numbers() {
        assert_eq!(format_duration(40_000), "40 s");
        assert_eq!(format_duration(12 * 60_000), "12 min");
        assert_eq!(format_duration(205 * 60_000), "3 h 25 min");
        assert_eq!(format_count(1234567), "1,234,567");
        assert_eq!(format_count(12), "12");
    }
}
