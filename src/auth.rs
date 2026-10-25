//! OAuth "Authorization Code with PKCE" login, as described at
//! <https://developer.spotify.com/documentation/web-api/tutorials/code-pkce-flow>.
//!
//! The browser is sent to Spotify's login page. After the user agrees, Spotify
//! redirects the browser to `http://127.0.0.1:<port>/...`, where a tiny local
//! web server picks up the one-time code and trades it for tokens.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use url::Url;

const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Redirect URI for the user's own developer app. It has to be added, exactly
/// like this, in the app's settings on developer.spotify.com.
pub const WEB_REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";
pub const WEB_SCOPES: &[&str] = &[
    "user-read-private",
    "playlist-read-private",
    "playlist-read-collaborative",
    "user-library-read",
    "playlist-modify-private",
    "playlist-modify-public",
];

/// Needed to make and change playlists. Logins from versions before 0.4.0 lack them.
pub const PLAYLIST_EDIT_SCOPES: &[&str] = &["playlist-modify-private", "playlist-modify-public"];

/// Spotify only lets its own client ID log in to the audio servers, so the
/// player logs in with the ID librespot uses, on the redirect URI registered for it.
pub const PLAYER_REDIRECT_URI: &str = "http://127.0.0.1:8898/login";
pub const PLAYER_SCOPES: &[&str] = &["streaming"];

/// How long to wait for the user to finish logging in in the browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Instant,
    /// The permissions the user granted, separated by spaces.
    pub scope: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(default)]
    scope: String,
}

impl From<TokenResponse> for Tokens {
    fn from(r: TokenResponse) -> Self {
        Self {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            // Refresh a minute early so a request never goes out with a token about to expire.
            expires_at: Instant::now() + Duration::from_secs(r.expires_in.saturating_sub(60)),
            scope: r.scope,
        }
    }
}

fn random_string(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
        .collect()
}

/// Runs the whole browser login and returns the tokens.
///
/// `show_url` is called with the login address, so the UI can show it in case
/// the browser does not open by itself.
pub async fn login(
    http: &reqwest::Client,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[&str],
    show_url: impl FnOnce(String),
) -> Result<Tokens> {
    let redirect = Url::parse(redirect_uri)?;
    let port = redirect.port().context("redirect URI has no port")?;

    let verifier = random_string(64);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let state = random_string(16);

    let mut auth_url = Url::parse(AUTHORIZE_URL)?;
    auth_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("scope", &scopes.join(" "))
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &challenge)
        .append_pair("state", &state);

    // Start listening before opening the browser, so the redirect cannot arrive too early.
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| {
            format!("Could not listen on 127.0.0.1:{port}. Is another program using that port?")
        })?;

    if let Err(e) = open::that_detached(auth_url.as_str()) {
        log::warn!("Could not open the browser: {e}");
    }
    show_url(auth_url.to_string());

    let code = tokio::time::timeout(
        LOGIN_TIMEOUT,
        wait_for_code(&listener, redirect.path(), &state),
    )
    .await
    .map_err(|_| anyhow!("Login timed out. Please try again."))??;

    let response = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", &verifier),
        ])
        .send()
        .await?;
    parse_token_response(response).await
}

/// Gets a new access token from a refresh token, without opening the browser.
pub async fn refresh(
    http: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<Tokens> {
    let response = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ])
        .send()
        .await?;
    let mut tokens = parse_token_response(response).await?;
    // Spotify sometimes hands out a new refresh token; if not, the old one stays valid.
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token.to_string());
    }
    Ok(tokens)
}

async fn parse_token_response(response: reqwest::Response) -> Result<Tokens> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        bail!("Spotify refused the login ({status}): {body}");
    }
    let parsed: TokenResponse = serde_json::from_str(&body).context("unexpected token response")?;
    Ok(parsed.into())
}

/// Accepts browser connections until the redirect with the login code arrives.
async fn wait_for_code(listener: &TcpListener, path: &str, state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut request_line = String::new();
        BufReader::new(&mut stream)
            .read_line(&mut request_line)
            .await?;

        // The request line looks like: "GET /callback?code=...&state=... HTTP/1.1"
        let target = request_line.split_whitespace().nth(1).unwrap_or("/");
        let url = Url::parse("http://127.0.0.1")?.join(target)?;
        if url.path() != path {
            // e.g. the browser asking for /favicon.ico
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                .await;
            continue;
        }

        let param = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };
        let result = if let Some(error) = param("error") {
            Err(anyhow!("Login was cancelled or refused: {error}"))
        } else if param("state").as_deref() != Some(state) {
            Err(anyhow!(
                "Login answer did not match the request. Please try again."
            ))
        } else {
            param("code").ok_or_else(|| anyhow!("Spotify did not send a login code"))
        };

        let message = match &result {
            Ok(_) => "Logged in. You can close this tab and go back to lite-spotify.",
            Err(_) => "Login failed. Go back to lite-spotify for details.",
        };
        let page = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>lite-spotify</title></head>\
             <body style=\"background:#121212;color:#eee;font-family:sans-serif;padding:3em\">\
             <h2>{message}</h2></body></html>"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
        return result;
    }
}
