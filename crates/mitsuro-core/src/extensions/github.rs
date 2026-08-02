//! GitHub API for extension binary downloads

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tracing::{debug, warn};

const GITHUB_API_URL: &str = "https://api.github.com";

#[derive(Deserialize, Debug, Clone)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(rename = "prerelease")]
    pub pre_release: bool,
    pub assets: Vec<GithubReleaseAsset>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GithubReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

/// Fetch the latest GitHub release for a repository
pub async fn latest_github_release(
    repo: &str,
    require_assets: bool,
    pre_release: bool,
    client: &reqwest::Client,
) -> Result<GithubRelease> {
    debug!("GitHub API: fetching releases for repo '{}'", repo);
    let url = format!("{GITHUB_API_URL}/repos/{repo}/releases");

    let mut request = client.get(&url).header("User-Agent", "mitsuro");

    // Use GITHUB_TOKEN if available (avoids rate limiting)
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    let response = request.send().await.context("fetching releases")?;

    if response.status().is_client_error() {
        warn!("GitHub API error for '{}': {}", repo, response.status());
        bail!("GitHub API error for '{}': {}", repo, response.status());
    }

    let releases: Vec<GithubRelease> = response.json().await.context("parsing releases")?;
    debug!(
        "GitHub API: found {} releases for '{}'",
        releases.len(),
        repo
    );

    releases
        .into_iter()
        .filter(|r| !require_assets || !r.assets.is_empty())
        .find(|r| r.pre_release == pre_release)
        .context(format!("no matching release found for '{}'", repo))
}

/// Fetch a specific GitHub release by tag name
pub async fn get_release_by_tag_name(
    repo: &str,
    tag: &str,
    client: &reqwest::Client,
) -> Result<GithubRelease> {
    let url = format!("{GITHUB_API_URL}/repos/{repo}/releases/tags/{tag}");

    let mut request = client.get(&url).header("User-Agent", "mitsuro");

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    let response = request.send().await.context("fetching release")?;

    if response.status().is_client_error() {
        bail!("GitHub API error: {}", response.status());
    }

    response.json().await.context("parsing release")
}
