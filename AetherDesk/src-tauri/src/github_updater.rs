use serde::Deserialize;

const RELEASES_API_URL: &str = "https://api.github.com/repos/michelegoku3/Aether/releases/latest";

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

pub struct GithubReleaseManager {
    client: reqwest::Client,
}

impl GithubReleaseManager {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Queries the public GitHub REST API to get the latest tag name and zip download URL
    pub async fn fetch_latest_release(&self) -> Result<(String, String), String> {
        // GitHub API requires a User-Agent header to prevent blocking
        let response = self.client.get(RELEASES_API_URL)
            .header("User-Agent", "AetherDesk-Updater")
            .send()
            .await
            .map_err(|e| format!("GitHub API network error: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("GitHub API returned HTTP error: {}", response.status()));
        }

        let release = response.json::<GithubRelease>().await
            .map_err(|e| format!("Failed to parse GitHub JSON: {}", e))?;

        // Search for the asset that contains "AetherDLL" or has a .zip extension
        let target_asset = release.assets.iter()
            .find(|asset| asset.name.to_lowercase().contains("aetherdll") || asset.name.ends_with(".zip"))
            .ok_or_else(|| "Could not find AetherDLL.zip asset in the latest GitHub release".to_string())?;

        Ok((release.tag_name, target_asset.browser_download_url.clone()))
    }
}
