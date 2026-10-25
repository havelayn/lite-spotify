//! Small client for the parts of the Spotify Web API the app needs:
//! playlists, Liked Songs, albums, podcasts and search.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::auth::{self, Tokens};

const API: &str = "https://api.spotify.com/v1";
/// Stop loading after this many tracks, to keep memory use small.
const MAX_TRACKS: usize = 10_000;
/// Development-mode apps may ask for at most 10 search results per type.
const SEARCH_LIMIT: &str = "10";

/// A song or podcast episode as the UI shows it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub uri: String,
    pub name: String,
    /// Artists, or the show's name for an episode.
    pub artists: String,
    /// Album name, or the release date for an episode.
    pub album: String,
    pub duration_ms: u32,
    /// Small album cover, used in the now-playing bar.
    pub image_url: Option<String>,
    pub playable: bool,
    pub is_episode: bool,
    /// Release year, 0 when unknown.
    pub year: u16,
    /// When the user saved it (Liked Songs) or added it to the playlist, as
    /// "YYYY-MM-DD…". Empty when unknown.
    pub added_at: String,
}

#[derive(Clone, Debug)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub owner: String,
    /// Spotify user id of the owner, to know which playlists the user may change.
    pub owner_id: String,
    pub total: u32,
}

/// Who is signed in.
#[derive(Clone, Debug)]
pub struct Me {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artists: String,
}

/// A podcast.
#[derive(Clone, Debug)]
pub struct Show {
    pub id: String,
    pub name: String,
    pub publisher: String,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub playlists: Vec<Playlist>,
    pub shows: Vec<Show>,
}

/// Where a list of tracks comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TrackSource {
    LikedSongs,
    Playlist {
        id: String,
        name: String,
    },
    Album {
        id: String,
        name: String,
    },
    /// The episodes of a podcast.
    Show {
        id: String,
        name: String,
    },
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
        let value = self
            .request(reqwest::Method::GET, path_or_url, query, None)
            .await?;
        serde_json::from_value(value)
            .with_context(|| format!("Unexpected answer from {path_or_url}"))
    }

    /// Sends a change (POST, PUT, DELETE) with a JSON body.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        self.request(method, path, &[], body).await
    }

    /// Makes a Web API request, refreshing the login and waiting out rate limits
    /// as needed. Returns the JSON answer (`null` when there is none).
    async fn request(
        &self,
        method: reqwest::Method,
        path_or_url: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value> {
        let url = if path_or_url.starts_with("https://") {
            path_or_url.to_string()
        } else {
            format!("{API}{path_or_url}")
        };
        let mut force_refresh = false;
        for _attempt in 0..4 {
            let token = self.access_token(force_refresh).await?;
            let mut request = self
                .http
                .request(method.clone(), &url)
                .query(query)
                .bearer_auth(token);
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = request.send().await?;
            let status = response.status();
            if status.is_success() {
                let body = response.bytes().await?;
                if body.iter().all(u8::is_ascii_whitespace) {
                    return Ok(Value::Null);
                }
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

    pub async fn me(&self) -> Result<Me> {
        let me: RawUser = self.get("/me", &[]).await?;
        Ok(Me {
            name: me.display_name.unwrap_or_else(|| me.id.clone()),
            id: me.id,
        })
    }

    /// Whether the login allows changing playlists and Liked Songs (logins from
    /// older versions don't).
    pub async fn can_edit(&self) -> bool {
        let tokens = self.tokens.lock().await;
        auth::EDIT_SCOPES
            .iter()
            .all(|s| tokens.scope.split(' ').any(|g| g == *s))
    }

    /// Adds songs or episodes to Liked Songs (`like`) or removes them.
    pub async fn set_liked(&self, uris: &[String], like: bool) -> Result<()> {
        let method = if like {
            reqwest::Method::PUT
        } else {
            reqwest::Method::DELETE
        };
        // At most 40 per request; the list goes in the address, not the body.
        for chunk in uris.chunks(40) {
            let list = chunk.join(",");
            self.request(method.clone(), "/me/library", &[("uris", &list)], None)
                .await?;
        }
        Ok(())
    }

    /// Whether each of `uris` (at most 40) is in Liked Songs.
    pub async fn liked(&self, uris: &[String]) -> Result<Vec<bool>> {
        let list = uris.join(",");
        self.get("/me/library/contains", &[("uris", &list)]).await
    }

    /// Makes a new private playlist.
    pub async fn create_playlist(&self, name: &str) -> Result<Playlist> {
        let body = serde_json::json!({
            "name": name,
            "public": false,
            "description": "Made with lite-spotify",
        });
        let value = self
            .send(reqwest::Method::POST, "/me/playlists", Some(&body))
            .await?;
        let raw: RawPlaylist = serde_json::from_value(value).context("Unexpected answer")?;
        Ok(raw.into_playlist())
    }

    /// Adds songs or episodes to the end of a playlist.
    pub async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        for chunk in uris.chunks(100) {
            let body = serde_json::json!({ "uris": chunk });
            self.send(
                reqwest::Method::POST,
                &format!("/playlists/{playlist_id}/items"),
                Some(&body),
            )
            .await?;
        }
        Ok(())
    }

    /// Removes every copy of a song from a playlist.
    pub async fn remove_from_playlist(&self, playlist_id: &str, uri: &str) -> Result<()> {
        let path = format!("/playlists/{playlist_id}/items");
        // The body key was renamed from "tracks" to "items" with the 2026 endpoint
        // rename; send the new name and fall back to the old one.
        let body = serde_json::json!({ "items": [{ "uri": uri }] });
        match self.send(reqwest::Method::DELETE, &path, Some(&body)).await {
            Err(e) if is_status(&e, reqwest::StatusCode::BAD_REQUEST) => {
                let body = serde_json::json!({ "tracks": [{ "uri": uri }] });
                self.send(reqwest::Method::DELETE, &path, Some(&body))
                    .await?;
                Ok(())
            }
            other => other.map(|_| ()),
        }
    }

    pub async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        let body = serde_json::json!({ "name": name });
        self.send(
            reqwest::Method::PUT,
            &format!("/playlists/{playlist_id}"),
            Some(&body),
        )
        .await?;
        Ok(())
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

    /// The podcasts the user follows.
    pub async fn saved_shows(&self) -> Result<Vec<Show>> {
        let mut out = Vec::new();
        let mut next = Some(format!("{API}/me/shows?limit=50"));
        while let Some(url) = next {
            let page: Paging = self.get(&url, &[]).await?;
            out.extend(parse_items::<RawSavedShow>(page.items).map(|s| s.show.into_show()));
            next = page.next;
        }
        Ok(out)
    }

    /// Loads every track of `source`, calling `on_page` after each page so the
    /// list can show up in the UI while the rest is still loading.
    pub async fn tracks(
        &self,
        source: &TrackSource,
        on_page: impl FnMut(Vec<Track>),
    ) -> Result<()> {
        self.tracks_limited(source, MAX_TRACKS, on_page).await
    }

    /// Like [`Self::tracks`], but stops after about `max` tracks.
    async fn tracks_limited(
        &self,
        source: &TrackSource,
        max: usize,
        mut on_page: impl FnMut(Vec<Track>),
    ) -> Result<()> {
        // "market=from_token" makes Spotify say whether each track can be played in
        // the user's country. The `next` links Spotify returns keep the parameter.
        let (mut next, parent) = match source {
            TrackSource::LikedSongs => (
                Some(format!("{API}/me/tracks?limit=50&market=from_token")),
                None,
            ),
            // additional_types=episode: without it, podcast episodes in playlists come back empty.
            TrackSource::Playlist { id, .. } => (
                Some(format!(
                    "{API}/playlists/{id}/items?limit=50&market=from_token&additional_types=track,episode"
                )),
                None,
            ),
            // Album tracks and show episodes leave out the album or show itself,
            // so fetch that first; it comes with the first page.
            TrackSource::Album { id, .. } | TrackSource::Show { id, .. } => {
                let path = match source {
                    TrackSource::Album { .. } => format!("/albums/{id}"),
                    _ => format!("/shows/{id}"),
                };
                let raw: RawAlbum = self.get(&path, &[("market", "from_token")]).await?;
                let page = raw
                    .tracks
                    .clone()
                    .or(raw.episodes.clone())
                    .unwrap_or_default();
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
            let tracks: Vec<Track> = if parent.is_some() {
                parse_items::<RawTrack>(page.items)
                    .filter_map(|t| t.into_track(parent.as_ref()))
                    .collect()
            } else {
                // Playlist and library entries wrap the track. Spotify renamed the
                // field from "track" to "item" in 2026, so accept both.
                parse_items::<RawEntry>(page.items)
                    .filter_map(RawEntry::into_track)
                    .collect()
            };
            count += tracks.len();
            on_page(tracks);
            next = if count >= max { None } else { page.next };
        }
        Ok(())
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        let raw: RawSearch = self
            .get(
                "/search",
                &[
                    ("q", query),
                    ("type", "track,album,playlist,show"),
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
            shows: raw
                .shows
                .map(|p| {
                    parse_items::<RawShow>(p.items)
                        .map(RawShow::into_show)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Loads up to `max` tracks of `source`, for the Fun tab's mixes.
    pub async fn sample_tracks(&self, source: &TrackSource, max: usize) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        self.tracks_limited(source, max, |page| out.extend(page))
            .await?;
        out.truncate(max);
        Ok(out)
    }

    /// Downloads an image (album covers are small JPEGs).
    pub async fn image(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.http.get(url).send().await?.error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    }
}

/// Whether `e` is a Web API error with this HTTP status.
pub fn is_status(e: &anyhow::Error, status: reqwest::StatusCode) -> bool {
    e.downcast_ref::<ApiError>()
        .is_some_and(|e| e.status == status)
}

/// Parses each entry on its own and skips the ones that do not fit (Spotify
/// sometimes sends `null` entries, local files, and so on).
fn parse_items<T: DeserializeOwned>(items: Vec<Value>) -> impl Iterator<Item = T> {
    items
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
}

pub fn join_names(names: impl IntoIterator<Item = String>) -> String {
    names.into_iter().collect::<Vec<_>>().join(", ")
}

/// The year at the start of a Spotify date ("2011", "2011-05" or "2011-05-20").
fn year_of(date: &str) -> u16 {
    date.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0)
}

/// Picks the smallest cover that is at least 300 pixels wide (Spotify's middle
/// size), which looks sharp in the now-playing panel.
fn small_image(images: &[RawImage]) -> Option<String> {
    images
        .iter()
        .filter(|i| i.width.unwrap_or(300) >= 300)
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
    added_at: Option<String>,
}

impl RawEntry {
    /// Playlist and library entries wrap the track. Spotify renamed the field
    /// from "track" to "item" in 2026, so accept both.
    fn into_track(self) -> Option<Track> {
        let raw: RawTrack = serde_json::from_value(self.item.or(self.track)?).ok()?;
        let mut track = raw.into_track(None)?;
        track.added_at = self.added_at.unwrap_or_default();
        Some(track)
    }
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

/// An album, or a podcast show (they are read the same way).
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
    #[serde(default)]
    release_date: String,
    tracks: Option<Paging>,
    /// Only on shows.
    episodes: Option<Paging>,
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
    // Only on podcast episodes:
    show: Option<RawShow>,
    #[serde(default)]
    images: Vec<RawImage>,
    #[serde(default)]
    release_date: String,
}

impl RawTrack {
    /// `parent` is the album (for album tracks) or show (for episodes) the item came from.
    fn into_track(self, parent: Option<&RawAlbum>) -> Option<Track> {
        let playable_uri = match self.kind.as_str() {
            "track" => "spotify:track:",
            "episode" => "spotify:episode:",
            _ => return None,
        };
        let playable =
            !self.is_local && self.is_playable != Some(false) && self.uri.starts_with(playable_uri);
        if self.kind == "episode" {
            let show_name = parent
                .map(|p| p.name.clone())
                .or_else(|| self.show.as_ref().map(|s| s.name.clone()))
                .unwrap_or_default();
            let image_url = small_image(&self.images)
                .or_else(|| parent.and_then(|p| small_image(&p.images)))
                .or_else(|| self.show.as_ref().and_then(|s| small_image(&s.images)));
            return Some(Track {
                uri: self.uri,
                name: self.name,
                artists: show_name,
                year: year_of(&self.release_date),
                album: self.release_date,
                duration_ms: self.duration_ms,
                image_url,
                playable,
                is_episode: true,
                added_at: String::new(),
            });
        }
        let album = parent.or(self.album.as_ref());
        Some(Track {
            playable,
            uri: self.uri,
            name: self.name,
            artists: join_names(self.artists.into_iter().map(|a| a.name)),
            album: album.map(|a| a.name.clone()).unwrap_or_default(),
            duration_ms: self.duration_ms,
            image_url: album.and_then(|a| small_image(&a.images)),
            is_episode: false,
            year: album.map(|a| year_of(&a.release_date)).unwrap_or(0),
            added_at: String::new(),
        })
    }
}

#[derive(Deserialize)]
struct RawSavedShow {
    show: RawShow,
}

#[derive(Deserialize)]
struct RawShow {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    /// Spotify stopped sending this to development-mode apps in 2026; may be missing.
    #[serde(default)]
    publisher: String,
    #[serde(default)]
    images: Vec<RawImage>,
}

impl RawShow {
    fn into_show(self) -> Show {
        Show {
            id: self.id,
            name: self.name,
            publisher: self.publisher,
        }
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
            owner_id: self
                .owner
                .as_ref()
                .map(|o| o.id.clone())
                .unwrap_or_default(),
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
    shows: Option<Paging>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_entries_accept_old_and_new_field_names() {
        let page: Paging = serde_json::from_str(
            r#"{"items":[
                {"added_at":"2020-01-01T00:00:00Z",
                 "item":{"type":"track","uri":"spotify:track:a","name":"New","artists":[{"name":"X"}],"duration_ms":1000,
                         "album":{"name":"Al","release_date":"1999-05-01","images":[{"url":"big","width":640},{"url":"mid","width":300},{"url":"small","width":64}]}}},
                {"track":{"type":"track","uri":"spotify:track:b","name":"Old","artists":[],"duration_ms":2000}},
                {"item":{"type":"episode","uri":"spotify:episode:c","name":"Pod","release_date":"2026-01-02",
                         "show":{"name":"My Show"}}},
                {"item":{"type":"chapter","uri":"spotify:chapter:d","name":"Book"}},
                null
            ],"next":null}"#,
        )
        .unwrap();
        let tracks: Vec<Track> = parse_items::<RawEntry>(page.items)
            .filter_map(RawEntry::into_track)
            .collect();
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].name, "New");
        assert_eq!(tracks[0].image_url.as_deref(), Some("mid"));
        assert_eq!(tracks[0].year, 1999);
        assert_eq!(tracks[0].added_at, "2020-01-01T00:00:00Z");
        assert_eq!(tracks[1].name, "Old");
        assert!(tracks[1].playable);
        assert!(tracks[2].is_episode && tracks[2].playable);
        assert_eq!(tracks[2].artists, "My Show");
        assert_eq!(tracks[2].album, "2026-01-02");
    }

    #[test]
    fn show_episodes_take_the_show_name() {
        let show: RawAlbum = serde_json::from_str(
            r#"{"id":"s","name":"The Show","images":[{"url":"cover","width":64}],
                "episodes":{"items":[{"type":"episode","uri":"spotify:episode:e","name":"Ep 1",
                "duration_ms":60000,"release_date":"2026-03-04"}],"next":null}}"#,
        )
        .unwrap();
        let page = show.episodes.clone().unwrap();
        let eps: Vec<Track> = parse_items::<RawTrack>(page.items)
            .filter_map(|t| t.into_track(Some(&show)))
            .collect();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].artists, "The Show");
        assert_eq!(eps[0].image_url.as_deref(), Some("cover"));
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
