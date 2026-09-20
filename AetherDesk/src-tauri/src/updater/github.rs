use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

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

/// The single definition of "this channel has no release" for the REST path.
/// Produced by [`GithubReleaseManager::fetch_latest_by_prefix_api`] only.
fn no_api_release_error(prefixes: &[&str]) -> String {
    format!("No published GitHub release found for prefixes {prefixes:?}")
}

/// Same, for the public (no-API) fallback path.
fn no_atom_release_error(prefixes: &[&str]) -> String {
    format!("No release atom entry found for prefixes {prefixes:?}")
}

/// True when a lookup failed because the channel simply has **no release**,
/// rather than because GitHub was unreachable or refused the call.
///
/// The distinction drives log severity (F6): probing an empty test channel is a
/// normal state — it is how every install without a `tdll-*` release behaves —
/// and reporting it as ERROR on every check trains users to ignore the log.
/// Real transport/HTTP failures stay errors. The recognised messages are the
/// ones produced by `no_api_release_error`/`no_atom_release_error` above, so
/// there is one definition of the text and one of its meaning.
pub fn channel_has_no_release(error: &str) -> bool {
    error.contains("No published GitHub release found for prefixes")
        || error.contains("No release atom entry found for prefixes")
}

/// Which test channel a prefix set belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestChannel {
    Dll,
    Desk,
}

impl TestChannel {
    fn from_prefixes(prefixes: &[&str]) -> Option<Self> {
        if prefixes == TDLL_TAG_PREFIXES {
            Some(TestChannel::Dll)
        } else if prefixes == TDESK_TAG_PREFIXES {
            Some(TestChannel::Desk)
        } else {
            None
        }
    }
}

/// How long "this test channel has no release" is believed before probing again.
///
/// A test-channel probe costs two API requests (REST + atom fallback) and ends
/// the same way on every check until a release is published. Remembering the
/// absence keeps the 60-requests/hour budget for the checks that can change.
const TEST_CHANNEL_ABSENCE_TTL: Duration = Duration::from_secs(15 * 60);

fn test_channel_absence() -> &'static Mutex<HashMap<TestChannel, Instant>> {
    static ABSENCE: OnceLock<Mutex<HashMap<TestChannel, Instant>>> = OnceLock::new();
    ABSENCE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// True when this test channel was recently observed to have no release.
pub fn test_channel_recently_empty(channel: TestChannel) -> bool {
    test_channel_absence()
        .lock()
        .map(|map| {
            map.get(&channel)
                .map(|seen| seen.elapsed() < TEST_CHANNEL_ABSENCE_TTL)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn note_test_channel_empty(channel: TestChannel) {
    if let Ok(mut map) = test_channel_absence().lock() {
        map.insert(channel, Instant::now());
    }
}

fn note_test_channel_published(channel: TestChannel) {
    if let Ok(mut map) = test_channel_absence().lock() {
        map.remove(&channel);
    }
}

// ============================================================================
// Update-lookup single flight (F7)
// ============================================================================

/// How long a completed lookup is reused before another network request.
///
/// The double IPC (React StrictMode, plus the DLL-change path running the check
/// alongside the settings save) issued 2 full checks per settings save, each
/// costing up to 6 GitHub requests against a 60/hour unauthenticated budget.
/// A minute of reuse collapses those into one.
const LOOKUP_TTL: Duration = Duration::from_secs(60);
/// Same, for an *empty channel* (e.g. no `tdll-*` published). Absence changes
/// on release day, so it is cached longer but not forever.
const ABSENT_LOOKUP_TTL: Duration = Duration::from_secs(30 * 60);
/// How long a follower waits for the in-flight owner before doing the lookup
/// itself. Bounded so a panicked/hung owner can never block the UI.
const FOLLOWER_WAIT: Duration = Duration::from_secs(25);

#[derive(Clone)]
struct CachedLookup {
    at: Instant,
    /// `Err` is stored too: a failing lookup must be deduplicated as well, or
    /// the retry storm simply moves to the error path.
    result: Result<GithubRelease, String>,
}

pub(crate) struct LookupSlot {
    cached: Mutex<Option<CachedLookup>>,
    /// True while one caller is performing the network lookup for this key.
    in_flight: AtomicBool,
    /// Signalled when the owner publishes its result.
    done: Notify,
}

fn lookup_registry() -> &'static Mutex<HashMap<String, Arc<LookupSlot>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<LookupSlot>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Test-only window on the lookup slot: the coalescing itself needs a network
/// `GithubReleaseManager` call, but the reuse policy (what a second caller is
/// served, and for how long) is pure state and is worth pinning down.
#[cfg(test)]
pub(crate) fn lookup_slot_for_tests(key: &str) -> Arc<LookupSlot> {
    lookup_slot(key)
}

#[cfg(test)]
pub(crate) fn fresh_for_tests(slot: &LookupSlot) -> Option<Result<GithubRelease, String>> {
    slot.fresh().map(|cached| cached.result)
}

#[cfg(test)]
pub(crate) fn publish_for_tests(slot: &LookupSlot, result: Result<GithubRelease, String>) {
    slot.publish(result);
}

fn lookup_slot(key: &str) -> Arc<LookupSlot> {
    let mut registry = lookup_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry
        .entry(key.to_string())
        .or_insert_with(|| {
            Arc::new(LookupSlot {
                cached: Mutex::new(None),
                in_flight: AtomicBool::new(false),
                done: Notify::new(),
            })
        })
        .clone()
}

impl LookupSlot {
    fn fresh(&self) -> Option<CachedLookup> {
        let guard = self
            .cached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.as_ref()?;
        let ttl = match &cached.result {
            Ok(_) => LOOKUP_TTL,
            Err(error) if channel_has_no_release(error) => ABSENT_LOOKUP_TTL,
            Err(_) => LOOKUP_TTL,
        };
        (cached.at.elapsed() < ttl).then(|| cached.clone())
    }

    fn publish(&self, result: Result<GithubRelease, String>) {
        {
            let mut guard = self
                .cached
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *guard = Some(CachedLookup {
                at: Instant::now(),
                result,
            });
        }
        self.in_flight.store(false, AtomicOrdering::SeqCst);
        self.done.notify_waiters();
    }
}

#[derive(Clone)]
struct ReleasesApiEtagCache {
    etag: Option<String>,
    releases: Vec<GithubRelease>,
    #[allow(dead_code)]
    saved_at: Instant,
}

fn releases_api_etag_cache() -> &'static Mutex<Option<ReleasesApiEtagCache>> {
    static CACHE: OnceLock<Mutex<Option<ReleasesApiEtagCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
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

    /// Single entry point for release lookups: one in-flight request per
    /// channel set, a short reuse window, and an origin label in every log line.
    ///
    /// `origin` names the caller (`settings-save`, `manual-check`, `startup`,
    /// `install`, …). It exists because the duplicated checks in the logs were
    /// indistinguishable: two identical pairs per settings save, with no way to
    /// tell which path issued which. Now the duplicate is served from the
    /// cache and the log says so.
    pub async fn fetch_latest_by_prefix_tagged(
        &self,
        prefixes: &[&str],
        origin: &str,
    ) -> Result<GithubRelease, String> {
        let key = prefixes.join(",");
        let slot = lookup_slot(&key);

        if let Some(cached) = slot.fresh() {
            crate::desk_log_info!(
                "updater",
                "Release lookup for {:?} served from cache (origin={}, age_ms={})",
                prefixes,
                origin,
                cached.at.elapsed().as_millis()
            );
            return cached.result;
        }

        // Single owner: whoever wins this race performs the network lookup; a
        // concurrent caller waits for that result instead of duplicating it.
        let owner = slot
            .in_flight
            .compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
            .is_ok();
        if !owner {
            crate::desk_log_info!(
                "updater",
                "Release lookup for {:?} already in flight; joining it (origin={})",
                prefixes,
                origin
            );
            let notified = slot.done.notified();
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(FOLLOWER_WAIT) => {
                    crate::desk_log_warn!(
                        "updater",
                        "In-flight release lookup for {:?} did not finish within {}s; doing the lookup directly (origin={})",
                        prefixes,
                        FOLLOWER_WAIT.as_secs(),
                        origin
                    );
                }
            }
            if let Some(cached) = slot.fresh() {
                return cached.result;
            }
            // The owner is gone (or slower than the wait window): fall through
            // and answer this caller directly rather than returning nothing.
        }

        crate::desk_log_info!(
            "updater",
            "Looking up latest release for prefixes {:?} via GitHub REST API (origin={})",
            prefixes,
            origin
        );
        let result = self.lookup_with_fallback(prefixes, origin).await;
        // Keep the session memo of "this test channel is empty" in sync, so a
        // probe nobody can answer stops costing API requests (F7).
        if let Some(channel) = TestChannel::from_prefixes(prefixes) {
            match &result {
                Ok(_) => note_test_channel_published(channel),
                Err(error) if channel_has_no_release(error) => note_test_channel_empty(channel),
                Err(_) => {}
            }
        }
        slot.publish(result.clone());
        result
    }

    /// Primary: GitHub REST API. On any failure, public fallback (no API).
    async fn lookup_with_fallback(
        &self,
        prefixes: &[&str],
        origin: &str,
    ) -> Result<GithubRelease, String> {
        match self.fetch_latest_by_prefix_api(prefixes).await {
            Ok(release) => {
                crate::desk_log_info!(
                    "updater",
                    "GitHub API ok for {:?}: tag={} assets={} draft={} prerelease={} (origin={})",
                    prefixes,
                    release.tag_name,
                    release.assets.len(),
                    release.draft,
                    release.prerelease,
                    origin
                );
                Ok(release)
            }
            Err(api_error) if channel_has_no_release(&api_error) => {
                crate::desk_log_info!(
                    "updater",
                    "No release on channel {:?} via GitHub API ({}). Confirming against the public feed (origin={}).",
                    prefixes,
                    api_error,
                    origin
                );
                match self.fetch_latest_by_prefix_public_fallback(prefixes).await {
                    Ok(release) => {
                        crate::desk_log_info!(
                            "updater",
                            "Public fallback ok for {:?}: tag={} assets={} (origin={})",
                            prefixes,
                            release.tag_name,
                            release.assets.len(),
                            origin
                        );
                        Ok(release)
                    }
                    Err(fallback_error) if channel_has_no_release(&fallback_error) => {
                        crate::desk_log_info!(
                            "updater",
                            "Channel {:?} is empty on both sources (no release published, origin={}).",
                            prefixes,
                            origin
                        );
                        Err(no_api_release_error(prefixes))
                    }
                    Err(fallback_error) => {
                        crate::desk_log_error!(
                            "updater",
                            "Update lookup failed for {:?}. API: {}. Fallback: {} (origin={})",
                            prefixes,
                            api_error,
                            fallback_error,
                            origin
                        );
                        Err(format!(
                            "GitHub API failed ({}) and public fallback failed ({})",
                            api_error, fallback_error
                        ))
                    }
                }
            }
            Err(api_error) => {
                crate::desk_log_warn!(
                    "updater",
                    "GitHub API failed for {:?}: {}. Trying public fallback (no API) (origin={}).",
                    prefixes,
                    api_error,
                    origin
                );
                match self.fetch_latest_by_prefix_public_fallback(prefixes).await {
                    Ok(release) => {
                        crate::desk_log_info!(
                            "updater",
                            "Public fallback ok for {:?}: tag={} assets={} (origin={})",
                            prefixes,
                            release.tag_name,
                            release.assets.len(),
                            origin
                        );
                        Ok(release)
                    }
                    Err(fallback_error) => {
                        crate::desk_log_error!(
                            "updater",
                            "Update lookup failed for {:?}. API: {}. Fallback: {} (origin={})",
                            prefixes,
                            api_error,
                            fallback_error,
                            origin
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
            .ok_or_else(|| no_api_release_error(prefixes))
    }

    async fn fetch_releases_api(&self) -> Result<Vec<GithubRelease>, String> {
        let cached_etag = releases_api_etag_cache()
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().and_then(|c| c.etag.clone()));

        crate::desk_log_info!(
            "updater",
            "GET {} (ETag: {})",
            RELEASES_API_URL,
            cached_etag.as_deref().unwrap_or("none")
        );

        let mut req = self
            .client
            .get(RELEASES_API_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");

        if let Some(ref etag) = cached_etag {
            req = req.header("If-None-Match", etag);
        }

        let response = req
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

        if status == reqwest::StatusCode::NOT_MODIFIED {
            if let Ok(guard) = releases_api_etag_cache().lock() {
                if let Some(ref cache) = *guard {
                    crate::desk_log_info!(
                        "updater",
                        "GitHub API 304 Not Modified — reused {} cached releases without consuming rate limit",
                        cache.releases.len()
                    );
                    return Ok(cache.releases.clone());
                }
            }
        }

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

        let response_etag = header_str(response.headers(), "etag");
        let releases = response
            .json::<Vec<GithubRelease>>()
            .await
            .map_err(|e| format!("Failed to parse GitHub releases JSON: {}", e))?;

        if let Ok(mut guard) = releases_api_etag_cache().lock() {
            *guard = Some(ReleasesApiEtagCache {
                etag: if response_etag != "unknown" {
                    Some(response_etag)
                } else {
                    None
                },
                releases: releases.clone(),
                saved_at: Instant::now(),
            });
        }

        Ok(releases)
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
            .ok_or_else(|| no_atom_release_error(prefixes))?;

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

    pub async fn fetch_latest_dll_release(&self, origin: &str) -> Result<(String, String), String> {
        let release = self.fetch_latest_by_prefix_tagged(DLL_TAG_PREFIXES, origin).await?;
        let url = Self::dll_zip_url(&release)?;
        Ok((release.tag_name, url))
    }

    pub async fn fetch_latest_desk_release(&self, origin: &str) -> Result<GithubRelease, String> {
        self.fetch_latest_by_prefix_tagged(DESK_TAG_PREFIXES, origin).await
    }

    pub async fn fetch_latest_desk_test_release(&self, origin: &str) -> Result<GithubRelease, String> {
        self.fetch_latest_by_prefix_tagged(TDESK_TAG_PREFIXES, origin).await
    }

    pub async fn fetch_latest_dll_test_release(&self, origin: &str) -> Result<(String, String), String> {
        let release = self.fetch_latest_by_prefix_tagged(TDLL_TAG_PREFIXES, origin).await?;
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
