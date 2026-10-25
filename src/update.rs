//! Looks on GitHub for a newer version of lite-spotify.

use anyhow::{Context, Result};
use serde::Deserialize;

const LATEST_RELEASE: &str = "https://api.github.com/repos/havelayn/lite-spotify/releases/latest";

/// A newer version that can be downloaded.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    /// The release page (what's new).
    pub page_url: String,
    /// The .exe download, or the release page if there is none.
    pub download_url: String,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
}

/// The latest release if it is newer than this version, `None` otherwise.
pub async fn check(http: &reqwest::Client) -> Result<Option<Release>> {
    let raw: RawRelease = http
        .get(LATEST_RELEASE)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("Unexpected answer from GitHub")?;
    let version = raw.tag_name.trim_start_matches('v').to_string();
    if !is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let download_url = raw
        .assets
        .into_iter()
        .find(|a| a.name.ends_with(".exe"))
        .map(|a| a.browser_download_url)
        .unwrap_or_else(|| raw.html_url.clone());
    Ok(Some(Release {
        version,
        page_url: raw.html_url,
        download_url,
    }))
}

/// Compares versions like "0.10.2" number by number.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|part| {
                part.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    parse(candidate) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
    }
}
