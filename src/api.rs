//! Small client for the parts of the Spotify Web API the app needs:
//! playlists, Liked Songs, albums and search.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::auth::{self, Tokens};

const API: &str = "https://api.spotify.com/v1";
/// Stop loading after this many tracks, to keep memory use small.
const MAX_TRACKS: usize = 10_000;
/// Development-mode apps may ask for at most 10 search results per type.
const SEARCH_LIMIT: &str = "10";

/// A track as the UI shows it.
#[derive(Clone, Debug)]
pub struct Track {
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub album: String,
    pub duration_ms: u32,
    /// Small album cover, used in the now-playing bar.
    pub image_url: Option<String>,
    pub playable: bool,
}

#[derive(Clone, Debug)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub total: u32,
}

#[derive(Clone, Debug)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artists: String,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub playlists: Vec<Playlist>,
}

/// Where a list of tracks comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackSource {
    LikedSongs,
    Playlist { id: String, name: String },
    Album { id: String, name: String },
}

/// An HTTP error answer from the Web API.
#[derive(Debug)]
pub struct ApiError {
    pub status: reqwest::StatusCode,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Spotify Web API error {}: {}", self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

pub struct WebApi {
    http: reqwest::Client,
    client_id: String,
    tokens: Mutex<Tokens>,
    /// Called whenever Spotify hands out a new refresh token, so it can be saved.
    on_new_refresh_token: Box<dyn Fn(String) + Send + Sync>,
}

impl WebApi {
    pub fn new(
        http: reqwest::Client,
        client_id: String,
        tokens: Tokens,
        on_new_refresh_token: impl Fn(String) + Send + Sync + 'static,
    ) -> Self {
        Self {
            http,
            client_id,
            tokens: Mutex::new(tokens),
            on_new_refresh_token: Box::new(on_new_refresh_token),
        }
    }

    async fn access_token(&self, force_refresh: bool) -> Result<String> {
        let mut tokens = self.tokens.lock().await;
        if force_refresh || std::time::Instant::now() >= tokens.expires_at {
            let refresh_token = tokens
                .refresh_token
                .clone()
                .context("Login expired, please sign in again")?;
            let new = auth::refresh(&self.http, &self.client_id, &refresh_token).await?;
            if new.refresh_token != tokens.refresh_token
                && let Some(rt) = &new.refresh_token
            {
                (self.on_new_refresh_token)(rt.clone());
            }
            *tokens = new;
        }
        Ok(tokens.access_token.clone())
    }

    /// GET a Web API path (or a full `next` URL) and parse the JSON answer.
    async fn get<T: DeserializeOwned>(
        &self,
        path_or_url: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let url = if path_or_url.starts_with("https://") {
            path_or_url.to_string()
        } else {
            format!("{API}{path_or_url}")
        };
        let mut force_refresh = false;
        for _attempt in 0..4 {
            let token = self.access_token(force_refresh).await?;
            let response = self
                .http
                .get(&url)
                .query(query)
                .bearer_auth(token)
                .send()
                .await?;
            let status = response.status();
            if status.is_success() {
                let body = response.bytes().await?;
                return serde_json::from_slice(&body)
                    .with_context(|| format!("Unexpected answer from {url}"));
            }
            if status == reqwest::StatusCode::UNAUTHORIZED && !force_refresh {
                force_refresh = true;
                continue;
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2)
                    .min(30);
                log::warn!("Rate limited by Spotify, waiting {wait}s");
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or(body);
            return Err(ApiError { status, message }.into());
        }
        Err(anyhow!("Spotify kept refusing the request to {url}"))
    }

    pub async fn user_name(&self) -> Result<String> {
        let me: RawUser = self.get("/me", &[]).await?;
        Ok(me.display_name.unwrap_or(me.id))
    }

    /// All playlists the user owns or follows.
    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        let mut out = Vec::new();
        let mut next = Some(format!("{API}/me/playlists?limit=50"));
        while let Some(url) = next {
            let page: Paging = self.get(&url, &[]).await?;
            out.extend(parse_items::<RawPlaylist>(page.items).map(RawPlaylist::into_playlist));
            next = page.next;
        }
        Ok(out)
    }

    /// Loads every track of `source`, calling `on_page` after each page so the
    /// list can show up in the UI while the rest is still loading.
    pub async fn tracks(
        &self,
        source: &TrackSource,
        mut on_page: impl FnMut(Vec<Track>),
    ) -> Result<()> {
        // "market=from_token" makes Spotify say whether each track can be played in
        // the user's country. The `next` links Spotify returns keep the parameter.
        let (mut next, album) = match source {
            TrackSource::LikedSongs => (
                Some(format!("{API}/me/tracks?limit=50&market=from_token")),
                None,
            ),
            TrackSource::Playlist { id, .. } => (
                Some(format!(
                    "{API}/playlists/{id}/items?limit=50&market=from_token"
                )),
                None,
            ),
            TrackSource::Album { id, .. } => {
                // Album tracks leave out the album itself, so fetch the album first.
                let raw: RawAlbum = self
                    .get(&format!("/albums/{id}"), &[("market", "from_token")])
                    .await?;
                let page = raw.tracks.clone().unwrap_or_default();
                let tracks: Vec<Track> = parse_items::<RawTrack>(page.items)
                    .filter_map(|t| t.into_track(Some(&raw)))
                    .collect();
                on_page(tracks);
                (page.next, Some(raw))
            }
        };

        let mut count = 0;
        while let Some(url) = next {
            let page: Paging = self.get(&url, &[]).await?;
            let tracks: Vec<Track> = if album.is_some() {
                parse_items::<RawTrack>(page.items)
                    .filter_map(|t| t.into_track(album.as_ref()))
                    .collect()
            } else {
                // Playlist and library entries wrap the track. Spotify renamed the
                // field from "track" to "item" in 2026, so accept both.
                parse_items::<RawEntry>(page.items)
                    .filter_map(|e| e.item.or(e.track))
                    .filter_map(|v| serde_json::from_value::<RawTrack>(v).ok())
                    .filter_map(|t| t.into_track(None))
                    .collect()
            };
            count += tracks.len();
            on_page(tracks);
            next = if count >= MAX_TRACKS { None } else { page.next };
        }
        Ok(())
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        let raw: RawSearch = self
            .get(
                "/search",
                &[
                    ("q", query),
                    ("type", "track,album,playlist"),
                    ("limit", SEARCH_LIMIT),
                    ("market", "from_token"),
                ],
            )
            .await?;
        Ok(SearchResults {
            tracks: raw
                .tracks
                .map(|p| {
                    parse_items::<RawTrack>(p.items)
                        .filter_map(|t| t.into_track(None))
                        .collect()
                })
                .unwrap_or_default(),
            albums: raw
                .albums
                .map(|p| {
                    parse_items::<RawAlbum>(p.items)
                        .map(RawAlbum::into_album)
                        .collect()
                })
                .unwrap_or_default(),
            playlists: raw
                .playlists
                .map(|p| {
                    parse_items::<RawPlaylist>(p.items)
                        .map(RawPlaylist::into_playlist)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Downloads an image (album covers are small JPEGs).
    pub async fn image(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.http.get(url).send().await?.error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    }
}

/// Parses each entry on its own and skips the ones that do not fit (Spotify
/// sometimes sends `null` entries, podcast episodes, and so on).
fn parse_items<T: DeserializeOwned>(items: Vec<Value>) -> impl Iterator<Item = T> {
    items
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
}

pub fn join_names(names: impl IntoIterator<Item = String>) -> String {
    names.into_iter().collect::<Vec<_>>().join(", ")
}

/// Picks the smallest cover that is still at least 64 pixels wide.
fn small_image(images: &[RawImage]) -> Option<String> {
    images
        .iter()
        .filter(|i| i.width.unwrap_or(64) >= 64)
        .min_by_key(|i| i.width.unwrap_or(u32::MAX))
        .or(images.first())
        .map(|i| i.url.clone())
}

// ---- Raw JSON shapes. Everything is optional so small API changes do not break loading. ----

#[derive(Deserialize, Default, Clone)]
struct Paging {
    #[serde(default)]
    items: Vec<Value>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct RawUser {
    #[serde(default)]
    id: String,
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct RawEntry {
    item: Option<Value>,
    track: Option<Value>,
}

#[derive(Deserialize, Clone)]
struct RawImage {
    url: String,
    width: Option<u32>,
}

#[derive(Deserialize, Clone)]
struct RawArtist {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize, Clone)]
struct RawAlbum {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    artists: Vec<RawArtist>,
    #[serde(default)]
    images: Vec<RawImage>,
    tracks: Option<Paging>,
}

impl RawAlbum {
    fn into_album(self) -> Album {
        Album {
            id: self.id,
            name: self.name,
            artists: join_names(self.artists.into_iter().map(|a| a.name)),
        }
    }
}

#[derive(Deserialize)]
struct RawTrack {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    uri: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    artists: Vec<RawArtist>,
    album: Option<RawAlbum>,
    #[serde(default)]
    duration_ms: u32,
    is_playable: Option<bool>,
    #[serde(default)]
    is_local: bool,
}

impl RawTrack {
    fn into_track(self, album_override: Option<&RawAlbum>) -> Option<Track> {
        if self.kind != "track" {
            return None; // e.g. a podcast episode in a playlist
        }
        let album = album_override.or(self.album.as_ref());
        Some(Track {
            playable: !self.is_local
                && self.is_playable != Some(false)
                && self.uri.starts_with("spotify:track:"),
            uri: self.uri,
            name: self.name,
            artists: join_names(self.artists.into_iter().map(|a| a.name)),
            album: album.map(|a| a.name.clone()).unwrap_or_default(),
            duration_ms: self.duration_ms,
            image_url: album.and_then(|a| small_image(&a.images)),
        })
    }
}

#[derive(Deserialize)]
struct RawCount {
    #[serde(default)]
    total: u32,
}

#[derive(Deserialize)]
struct RawOwner {
    display_name: Option<String>,
    #[serde(default)]
    id: String,
}

#[derive(Deserialize)]
struct RawPlaylist {
    id: String,
    #[serde(default)]
    name: String,
    owner: Option<RawOwner>,
    // Renamed from "tracks" to "items" in 2026; accept both.
    items: Option<RawCount>,
    tracks: Option<RawCount>,
}

impl RawPlaylist {
    fn into_playlist(self) -> Playlist {
        Playlist {
            id: self.id,
            name: self.name,
            owner: self
                .owner
                .map(|o| o.display_name.unwrap_or(o.id))
                .unwrap_or_default(),
            total: self.items.or(self.tracks).map(|c| c.total).unwrap_or(0),
        }
    }
}

#[derive(Deserialize)]
struct RawSearch {
    tracks: Option<Paging>,
    albums: Option<Paging>,
    playlists: Option<Paging>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_entries_accept_old_and_new_field_names() {
        let page: Paging = serde_json::from_str(
            r#"{"items":[
                {"item":{"type":"track","uri":"spotify:track:a","name":"New","artists":[{"name":"X"}],"duration_ms":1000,
                         "album":{"name":"Al","images":[{"url":"big","width":640},{"url":"small","width":64}]}}},
                {"track":{"type":"track","uri":"spotify:track:b","name":"Old","artists":[],"duration_ms":2000}},
                {"item":{"type":"episode","uri":"spotify:episode:c","name":"Pod"}},
                null
            ],"next":null}"#,
        )
        .unwrap();
        let tracks: Vec<Track> = parse_items::<RawEntry>(page.items)
            .filter_map(|e| e.item.or(e.track))
            .filter_map(|v| serde_json::from_value::<RawTrack>(v).ok())
            .filter_map(|t| t.into_track(None))
            .collect();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "New");
        assert_eq!(tracks[0].image_url.as_deref(), Some("small"));
        assert_eq!(tracks[1].name, "Old");
        assert!(tracks[1].playable);
    }

    #[test]
    fn playlist_total_accepts_old_and_new_field_names() {
        let new: RawPlaylist =
            serde_json::from_str(r#"{"id":"1","name":"A","items":{"total":5}}"#).unwrap();
        let old: RawPlaylist =
            serde_json::from_str(r#"{"id":"2","name":"B","tracks":{"total":7}}"#).unwrap();
        assert_eq!(new.into_playlist().total, 5);
        assert_eq!(old.into_playlist().total, 7);
    }
}
