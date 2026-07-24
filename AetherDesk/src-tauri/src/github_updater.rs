use serde::Deserialize;

const RELEASES_API_URL: &str = "https://api.github.com/repos/michelegoku3/Aether/releases?per_page=100";
const DLL_TAG_PREFIXES: &[&str] = &["dll-", "dll-v"];
const DESK_TAG_PREFIXES: &[&str] = &["desk-", "desk-v"];

#[derive(Debug, Clone, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ComponentUpdateInfo {
    pub installed_version: String,
    pub latest_version: String,
    pub latest_tag: String,
    pub update_available: bool,
    pub release_url: String,
    pub notes: String,
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

    async fn fetch_releases(&self) -> Result<Vec<GithubRelease>, String> {
        let response = self
            .client
            .get(RELEASES_API_URL)
            .header("User-Agent", "AetherDesk-Updater")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| format!("GitHub API network error: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("GitHub API returned HTTP error: {}", response.status()));
        }

        response
            .json::<Vec<GithubRelease>>()
            .await
            .map_err(|e| format!("Failed to parse GitHub releases JSON: {}", e))
    }

    fn tag_has_prefix(tag: &str, prefixes: &[&str]) -> bool {
        let lower = tag.to_lowercase();
        prefixes.iter().any(|prefix| lower.starts_with(prefix))
    }

    pub fn component_version_from_tag(tag: &str) -> String {
        let lower = tag.to_lowercase();
        let without_component = if let Some(rest) = lower.strip_prefix("desk-") {
            rest
        } else if let Some(rest) = lower.strip_prefix("dll-") {
            rest
        } else {
            lower.as_str()
        };
        without_component.strip_prefix('v').unwrap_or(without_component).to_string()
    }

    pub fn tags_are_different_versions(installed: &str, latest_tag: &str) -> bool {
        let installed_norm = Self::normalize_version(installed);
        let latest_norm = Self::normalize_version(&Self::component_version_from_tag(latest_tag));
        installed_norm != latest_norm
    }

    fn normalize_version(version: &str) -> String {
        let lower = version.trim().to_ascii_lowercase();
        lower
            .trim_start_matches("desk-")
            .trim_start_matches("dll-")
            .trim_start_matches('v')
            .to_string()
    }

    pub async fn fetch_latest_by_prefix(&self, prefixes: &[&str]) -> Result<GithubRelease, String> {
        let releases = self.fetch_releases().await?;
        releases
            .into_iter()
            .find(|release| {
                !release.draft && !release.prerelease && Self::tag_has_prefix(&release.tag_name, prefixes)
            })
            .ok_or_else(|| format!("No published GitHub release found for prefixes {:?}", prefixes))
    }

    /// Latest AetherDLL release. It intentionally ignores AetherDesk releases.
    pub async fn fetch_latest_dll_release(&self) -> Result<(String, String), String> {
        let release = self.fetch_latest_by_prefix(DLL_TAG_PREFIXES).await?;

        let target_asset = release
            .assets
            .iter()
            .find(|asset| {
                let name = asset.name.to_lowercase();
                (name.contains("aetherdll") || name.contains("dll")) && name.ends_with(".zip")
            })
            .or_else(|| release.assets.iter().find(|asset| asset.name.to_lowercase().ends_with(".zip")))
            .ok_or_else(|| format!("Could not find AetherDLL .zip asset in release {}", release.tag_name))?;

        Ok((release.tag_name, target_asset.browser_download_url.clone()))
    }

    /// Latest AetherDesk release. It intentionally ignores AetherDLL releases.
    pub async fn fetch_latest_desk_release(&self) -> Result<GithubRelease, String> {
        self.fetch_latest_by_prefix(DESK_TAG_PREFIXES).await
    }

    /// Finds the Tauri updater manifest generated for the selected AetherDesk release.
    /// The preferred asset name is latest.json; latest-desk.json is also accepted.
    pub fn find_desk_updater_manifest_url(release: &GithubRelease) -> Result<String, String> {
        release
            .assets
            .iter()
            .find(|asset| {
                let name = asset.name.to_lowercase();
                name == "latest.json" || name == "latest-desk.json" || name.ends_with("latest.json")
            })
            .map(|asset| asset.browser_download_url.clone())
            .ok_or_else(|| {
                format!(
                    "Could not find latest.json/latest-desk.json asset in AetherDesk release {}",
                    release.tag_name
                )
            })
    }

    pub fn build_desk_update_info(current_version: String, release: &GithubRelease) -> ComponentUpdateInfo {
        let latest_version = Self::component_version_from_tag(&release.tag_name);
        let update_available = Self::tags_are_different_versions(&current_version, &release.tag_name);

        ComponentUpdateInfo {
            installed_version: current_version,
            latest_version,
            latest_tag: release.tag_name.clone(),
            update_available,
            release_url: release.html_url.clone().unwrap_or_default(),
            notes: release.body.clone().unwrap_or_default(),
        }
    }
}
