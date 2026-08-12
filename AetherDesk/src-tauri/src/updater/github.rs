use regex::Regex;
use serde::Deserialize;

const RELEASES_API_URL: &str = "https://api.github.com/repos/michelegoku3/Aether/releases?per_page=100";
const RELEASES_ATOM_URL: &str = "https://github.com/michelegoku3/Aether/releases.atom";
const REPO_OWNER: &str = "michelegoku3";
const REPO_NAME: &str = "Aether";
const USER_AGENT: &str = "AetherDesk-Updater";

const DLL_TAG_PREFIXES: &[&str] = &["dll-", "dll-v"];
const DESK_TAG_PREFIXES: &[&str] = &["desk-", "desk-v"];
const TDLL_TAG_PREFIXES: &[&str] = &["tdll-", "tdll-v"];
const TDESK_TAG_PREFIXES: &[&str] = &["tdesk-", "tdesk-v"];

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
    pub is_test: bool,
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

    fn tag_has_prefix(tag: &str, prefixes: &[&str]) -> bool {
        let lower = tag.to_lowercase();
        prefixes.iter().any(|prefix| lower.starts_with(prefix))
    }

    /// Returns the version portion of a tag, stripping the component and
    /// optional `v` prefix. Handles both stable (`desk-`, `dll-`) and test
    /// (`tdesk-`, `tdll-`) streams.
    pub fn component_version_from_tag(tag: &str) -> String {
        let lower = tag.to_lowercase();
        let without_component = if let Some(rest) = lower.strip_prefix("tdesk-") {
            rest
        } else if let Some(rest) = lower.strip_prefix("tdll-") {
            rest
        } else if let Some(rest) = lower.strip_prefix("desk-") {
            rest
        } else if let Some(rest) = lower.strip_prefix("dll-") {
            rest
        } else {
            lower.as_str()
        };
        without_component
            .strip_prefix('v')
            .unwrap_or(without_component)
            .to_string()
    }

    pub fn is_test_tag(tag: &str) -> bool {
        Self::tag_has_prefix(tag, TDESK_TAG_PREFIXES) || Self::tag_has_prefix(tag, TDLL_TAG_PREFIXES)
    }

    pub fn display_version_from_tag(tag_or_version: &str) -> String {
        let version = Self::component_version_from_tag(tag_or_version);
        if version.trim().is_empty() || version == "n/a" {
            "N/A".to_string()
        } else {
            format!("v{}", version)
        }
    }

    /// True only when the GitHub tag represents a version NEWER than the
    /// installed one.
    pub fn latest_is_newer_than(installed: &str, latest_tag: &str) -> bool {
        let installed_norm = Self::normalize_version(installed);
        let latest_norm = Self::component_version_from_tag(latest_tag);
        if installed_norm.is_empty()
            || latest_norm.is_empty()
            || installed_norm.eq_ignore_ascii_case("n/a")
            || latest_norm.eq_ignore_ascii_case("n/a")
        {
            return false;
        }
        Self::compare_version_tags(&latest_norm, &installed_norm) == std::cmp::Ordering::Greater
    }

    fn normalize_version(version: &str) -> String {
        let lower = version.trim().to_ascii_lowercase();
        lower
            .trim_start_matches("tdesk-")
            .trim_start_matches("tdll-")
            .trim_start_matches("desk-")
            .trim_start_matches("dll-")
            .trim_start_matches('v')
            .to_string()
    }

    fn version_sort_key_from_tag(tag: &str) -> Vec<u64> {
        Self::component_version_from_tag(tag)
            .split('.')
            .map(|part| {
                part.chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    }

    fn compare_version_tags(a: &str, b: &str) -> std::cmp::Ordering {
        let a_key = Self::version_sort_key_from_tag(a);
        let b_key = Self::version_sort_key_from_tag(b);
        let max_len = a_key.len().max(b_key.len());

        for index in 0..max_len {
            let a_part = *a_key.get(index).unwrap_or(&0);
            let b_part = *b_key.get(index).unwrap_or(&0);
            match a_part.cmp(&b_part) {
                std::cmp::Ordering::Equal => continue,
                ordering => return ordering,
            }
        }

        std::cmp::Ordering::Equal
    }

    /// Primary: GitHub REST API. On any failure, public fallback (no API).
    pub async fn fetch_latest_by_prefix(&self, prefixes: &[&str]) -> Result<GithubRelease, String> {
        crate::desk_log_info!(
            "updater",
            "Looking up latest release for prefixes {:?} via GitHub REST API",
            prefixes
        );
        match self.fetch_latest_by_prefix_api(prefixes).await {
            Ok(release) => {
                crate::desk_log_info!(
                    "updater",
                    "GitHub API ok for {:?}: tag={} assets={} draft={} prerelease={}",
                    prefixes,
                    release.tag_name,
                    release.assets.len(),
                    release.draft,
                    release.prerelease
                );
                Ok(release)
            }
            Err(api_error) => {
                crate::desk_log_error!(
                    "updater",
                    "GitHub API failed for {:?}: {}. Trying public fallback (no API).",
                    prefixes,
                    api_error
                );
                match self.fetch_latest_by_prefix_public_fallback(prefixes).await {
                    Ok(release) => {
                        crate::desk_log_info!(
                            "updater",
                            "Public fallback ok for {:?}: tag={} assets={}",
                            prefixes,
                            release.tag_name,
                            release.assets.len()
                        );
                        Ok(release)
                    }
                    Err(fallback_error) => {
                        crate::desk_log_error!(
                            "updater",
                            "Update lookup failed for {:?}. API: {}. Fallback: {}",
                            prefixes,
                            api_error,
                            fallback_error
                        );
                        Err(format!(
                            "GitHub API failed ({}) and public fallback failed ({})",
                            api_error, fallback_error
                        ))
                    }
                }
            }
        }
    }

    async fn fetch_latest_by_prefix_api(&self, prefixes: &[&str]) -> Result<GithubRelease, String> {
        let releases = self.fetch_releases_api().await?;
        crate::desk_log_debug!(
            "updater",
            "GitHub API returned {} release(s); filtering prefixes {:?}",
            releases.len(),
            prefixes
        );
        let allow_prerelease = prefixes
            .iter()
            .any(|prefix| prefix.starts_with('t'));
        releases
            .into_iter()
            .filter(|release| {
                if release.draft {
                    crate::desk_log_debug!("updater", "Skipping draft release {}", release.tag_name);
                    return false;
                }
                if release.prerelease && !allow_prerelease {
                    crate::desk_log_debug!(
                        "updater",
                        "Skipping prerelease {} (stable stream)",
                        release.tag_name
                    );
                    return false;
                }
                Self::tag_has_prefix(&release.tag_name, prefixes)
            })
            .max_by(|a, b| Self::compare_version_tags(&a.tag_name, &b.tag_name))
            .ok_or_else(|| format!("No published GitHub release found for prefixes {:?}", prefixes))
    }

    async fn fetch_releases_api(&self) -> Result<Vec<GithubRelease>, String> {
        crate::desk_log_info!("updater", "GET {}", RELEASES_API_URL);
        let response = self
            .client
            .get(RELEASES_API_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| format!("GitHub API network error: {}", e))?;

        let status = response.status();
        let remaining = header_str(response.headers(), "x-ratelimit-remaining");
        let limit = header_str(response.headers(), "x-ratelimit-limit");
        let reset = header_str(response.headers(), "x-ratelimit-reset");
        crate::desk_log_info!(
            "updater",
            "GitHub API status={} rate_limit={}/{} rate_reset={}",
            status,
            remaining,
            limit,
            reset
        );
        if remaining != "unknown" {
            if let Ok(left) = remaining.parse::<u32>() {
                if left == 0 {
                    crate::desk_log_error!(
                        "updater",
                        "GitHub API rate limit exhausted (0/{}). Resets at unix {}",
                        limit,
                        reset
                    );
                } else if left <= 10 {
                    crate::desk_log_warn!(
                        "updater",
                        "GitHub API rate limit nearly exhausted: {}/{} remaining (reset {})",
                        remaining,
                        limit,
                        reset
                    );
                }
            }
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let preview: String = body.chars().take(400).collect();
            return Err(format!(
                "GitHub API HTTP {}. rate_remaining={}, rate_reset={}, body={}",
                status, remaining, reset, preview
            ));
        }

        response
            .json::<Vec<GithubRelease>>()
            .await
            .map_err(|e| format!("Failed to parse GitHub releases JSON: {}", e))
    }

    /// No REST API: Atom feed for tags + conventional download URLs from CI names.
    async fn fetch_latest_by_prefix_public_fallback(
        &self,
        prefixes: &[&str],
    ) -> Result<GithubRelease, String> {
        crate::desk_log_info!("updater", "GET {} (public fallback, no API)", RELEASES_ATOM_URL);
        let atom = self
            .client
            .get(RELEASES_ATOM_URL)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| format!("GitHub releases.atom network error: {}", e))?;

        let status = atom.status();
        crate::desk_log_info!("updater", "releases.atom HTTP {}", status);
        if !status.is_success() {
            return Err(format!("GitHub releases.atom returned HTTP error: {}", status));
        }

        let atom_text = atom
            .text()
            .await
            .map_err(|e| format!("Failed to read GitHub releases.atom: {}", e))?;
        let tags = Self::release_tags_from_atom(&atom_text);
        crate::desk_log_info!(
            "updater",
            "releases.atom parsed {} tag(s): {}",
            tags.len(),
            tags.join(", ")
        );
        let tag_name = tags
            .into_iter()
            .filter(|tag| Self::tag_has_prefix(tag, prefixes))
            .max_by(|a, b| Self::compare_version_tags(a, b))
            .ok_or_else(|| format!("No release atom entry found for prefixes {:?}", prefixes))?;

        let html_url = format!(
            "https://github.com/{}/{}/releases/tag/{}",
            REPO_OWNER, REPO_NAME, tag_name
        );
        let assets = Self::conventional_assets(&tag_name);
        crate::desk_log_info!(
            "updater",
            "Fallback constructed {} conventional asset URL(s) for {}",
            assets.len(),
            tag_name
        );
        if assets.is_empty() {
            return Err(format!("No conventional assets for tag {}", tag_name));
        }

        Ok(GithubRelease {
            tag_name,
            body: None,
            html_url: Some(html_url),
            draft: false,
            prerelease: false,
            assets,
        })
    }

    pub fn release_tags_from_atom(atom: &str) -> Vec<String> {
        let Ok(re) = Regex::new(&format!(
            r#"(?:https://github\.com)?/{}/{}/releases/tag/([^"'<>?\s]+)"#,
            regex::escape(REPO_OWNER),
            regex::escape(REPO_NAME)
        )) else {
            return Vec::new();
        };

        let mut tags: Vec<String> = re
            .captures_iter(atom)
            .filter_map(|captures| captures.get(1).map(|tag| html_unescape(tag.as_str())))
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }

    /// Known CI asset names from `.github/workflows/build.yml`.
    pub fn conventional_assets(tag_name: &str) -> Vec<GithubAsset> {
        let version = Self::component_version_from_tag(tag_name);
        let lower = tag_name.to_ascii_lowercase();
        let mut assets = Vec::new();

        if lower.starts_with("tdesk-") || lower.starts_with("desk-") {
            let name = format!("AetherDesk-{}.zip", version);
            assets.push(Self::download_asset(&name, tag_name));
        } else if lower.starts_with("tdll-") || lower.starts_with("dll-") {
            let name = format!("AetherDLL-{}.zip", version);
            assets.push(Self::download_asset(&name, tag_name));
        }

        assets
    }

    fn download_asset(name: &str, tag_name: &str) -> GithubAsset {
        GithubAsset {
            name: name.to_string(),
            browser_download_url: format!(
                "https://github.com/{}/{}/releases/download/{}/{}",
                REPO_OWNER, REPO_NAME, tag_name, name
            ),
        }
    }

    pub async fn fetch_latest_dll_release(&self) -> Result<(String, String), String> {
        let release = self.fetch_latest_by_prefix(DLL_TAG_PREFIXES).await?;
        let url = Self::dll_zip_url(&release)?;
        Ok((release.tag_name, url))
    }

    pub async fn fetch_latest_desk_release(&self) -> Result<GithubRelease, String> {
        self.fetch_latest_by_prefix(DESK_TAG_PREFIXES).await
    }

    pub async fn fetch_latest_desk_test_release(&self) -> Result<GithubRelease, String> {
        self.fetch_latest_by_prefix(TDESK_TAG_PREFIXES).await
    }

    pub async fn fetch_latest_dll_test_release(&self) -> Result<(String, String), String> {
        let release = self.fetch_latest_by_prefix(TDLL_TAG_PREFIXES).await?;
        let url = Self::dll_zip_url(&release)?;
        Ok((release.tag_name, url))
    }

    fn dll_zip_url(release: &GithubRelease) -> Result<String, String> {
        release
            .assets
            .iter()
            .find(|asset| {
                let name = asset.name.to_lowercase();
                (name.contains("aetherdll") || name.contains("dll")) && name.ends_with(".zip")
            })
            .or_else(|| {
                release
                    .assets
                    .iter()
                    .find(|asset| asset.name.to_lowercase().ends_with(".zip"))
            })
            .map(|asset| asset.browser_download_url.clone())
            .ok_or_else(|| {
                format!(
                    "Could not find AetherDLL .zip asset in release {}",
                    release.tag_name
                )
            })
    }

    pub fn find_desk_zip_asset(release: &GithubRelease) -> Result<GithubAsset, String> {
        release
            .assets
            .iter()
            .find(|asset| {
                let lower = asset.name.to_lowercase();
                (lower.contains("aetherdesk") || lower.contains("desk")) && lower.ends_with(".zip")
            })
            .or_else(|| {
                release
                    .assets
                    .iter()
                    .find(|asset| asset.name.to_lowercase().ends_with(".zip"))
            })
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Could not find a portable .zip asset in AetherDesk release {}",
                    release.tag_name
                )
            })
    }

    pub fn build_desk_update_info(current_version: String, release: &GithubRelease) -> ComponentUpdateInfo {
        let latest_version = Self::component_version_from_tag(&release.tag_name);
        let update_available = Self::latest_is_newer_than(&current_version, &release.tag_name);

        ComponentUpdateInfo {
            installed_version: current_version,
            latest_version,
            latest_tag: release.tag_name.clone(),
            update_available,
            is_test: Self::is_test_tag(&release.tag_name),
            release_url: release.html_url.clone().unwrap_or_default(),
            notes: release.body.clone().unwrap_or_default(),
        }
    }

    pub fn build_desk_test_update_info(current_version: String, release: &GithubRelease) -> ComponentUpdateInfo {
        let latest_version = Self::component_version_from_tag(&release.tag_name);
        let update_available = Self::latest_is_newer_than(&current_version, &release.tag_name);
        ComponentUpdateInfo {
            installed_version: current_version,
            latest_version,
            latest_tag: release.tag_name.clone(),
            update_available,
            is_test: true,
            release_url: release.html_url.clone().unwrap_or_default(),
            notes: release.body.clone().unwrap_or_default(),
        }
    }
}

fn header_str(headers: &reqwest::header::HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#x2F;", "/")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
}
