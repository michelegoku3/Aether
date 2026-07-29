use regex::Regex;

const DLL_TAG_PREFIXES: &[&str] = &["dll-", "dll-v"];
const DESK_TAG_PREFIXES: &[&str] = &["desk-", "desk-v"];

#[derive(Debug, Clone)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone)]
pub struct GithubRelease {
    pub tag_name: String,
    pub body: Option<String>,
    pub html_url: Option<String>,
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
        without_component
            .strip_prefix('v')
            .unwrap_or(without_component)
            .to_string()
    }

    pub fn display_version_from_tag(tag_or_version: &str) -> String {
        let version = Self::component_version_from_tag(tag_or_version);
        if version.trim().is_empty() || version == "n/a" {
            "N/A".to_string()
        } else {
            format!("v{}", version)
        }
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

    pub async fn fetch_latest_by_prefix(&self, prefixes: &[&str]) -> Result<GithubRelease, String> {
        self.fetch_latest_by_prefix_public_fallback(prefixes).await
    }

    async fn fetch_latest_by_prefix_public_fallback(
        &self,
        prefixes: &[&str],
    ) -> Result<GithubRelease, String> {
        let atom = self
            .client
            .get("https://github.com/michelegoku3/Aether/releases.atom")
            .header("User-Agent", "AetherDesk-Updater")
            .send()
            .await
            .map_err(|e| format!("GitHub releases.atom network error: {}", e))?;

        if !atom.status().is_success() {
            return Err(format!("GitHub releases.atom returned HTTP error: {}", atom.status()));
        }

        let atom_text = atom
            .text()
            .await
            .map_err(|e| format!("Failed to read GitHub releases.atom: {}", e))?;
        let tags = Self::release_tags_from_atom(&atom_text);
        let tag_name = tags
            .into_iter()
            .filter(|tag| Self::tag_has_prefix(tag, prefixes))
            .max_by(|a, b| Self::compare_version_tags(a, b))
            .ok_or_else(|| format!("No release atom entry found for prefixes {:?}", prefixes))?;

        let html_url = format!("https://github.com/michelegoku3/Aether/releases/tag/{}", tag_name);
        let assets = self.fetch_release_assets_from_html(&tag_name, &html_url).await?;

        Ok(GithubRelease {
            tag_name,
            body: None,
            html_url: Some(html_url),
            assets,
        })
    }

    fn release_tags_from_atom(atom: &str) -> Vec<String> {
        let Ok(re) = Regex::new(r#"https://github\.com/michelegoku3/Aether/releases/tag/([^"<]+)"#) else {
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

    async fn fetch_release_assets_from_html(
        &self,
        tag_name: &str,
        html_url: &str,
    ) -> Result<Vec<GithubAsset>, String> {
        let expanded_assets_url = format!(
            "https://github.com/michelegoku3/Aether/releases/expanded_assets/{}",
            tag_name
        );
        let response = self
            .client
            .get(&expanded_assets_url)
            .header("User-Agent", "AetherDesk-Updater")
            .header("Accept", "text/html")
            .send()
            .await
            .map_err(|e| format!("GitHub expanded assets network error: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "GitHub expanded assets returned HTTP error: {}",
                response.status()
            ));
        }

        let html = response
            .text()
            .await
            .map_err(|e| format!("Failed to read GitHub expanded assets: {}", e))?;
        let escaped_tag = regex::escape(tag_name);
        let pattern = format!(
            r#"href=["'](?P<href>/michelegoku3/Aether/releases/download/{}/(?P<name>[^"'?#]+))"#,
            escaped_tag
        );
        let re = Regex::new(&pattern).map_err(|e| format!("Internal release asset regex error: {}", e))?;

        let mut assets = Vec::new();
        for captures in re.captures_iter(&html) {
            let Some(href) = captures.name("href") else { continue; };
            let Some(name) = captures.name("name") else { continue; };
            let asset_name = html_unescape(name.as_str());
            let browser_download_url = format!("https://github.com{}", html_unescape(href.as_str()));
            if !assets.iter().any(|asset: &GithubAsset| asset.name == asset_name) {
                assets.push(GithubAsset {
                    name: asset_name,
                    browser_download_url,
                });
            }
        }

        if assets.is_empty() {
            return Err(format!(
                "No downloadable assets found on GitHub expanded assets page {} (release {})",
                expanded_assets_url, html_url
            ));
        }

        Ok(assets)
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

fn html_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#x2F;", "/")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
}
