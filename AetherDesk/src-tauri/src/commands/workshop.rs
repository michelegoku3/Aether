use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use serde::Serialize;
use tokio::sync::Mutex;

use crate::core::paths::LocalAppPaths;
use crate::core::settings::load_settings;
use crate::providers::hubcap::HubcapClient;

fn workshop_cache_path(workshop_id: u64) -> PathBuf {
    LocalAppPaths::temp_dir()
        .join("hubcap")
        .join("workshop")
        .join(format!("{workshop_id}.manifest"))
}

fn cached_workshop_manifest_matches(workshop_id: u64, expected_gid: u64) -> bool {
    let path = workshop_cache_path(workshop_id);
    path.is_file()
        && std::fs::metadata(&path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
        && std::fs::read(path)
            .ok()
            .and_then(|bytes| manifest_identity(&bytes).ok())
            .map(|(_, gid)| gid == expected_gid)
            .unwrap_or(false)
}

/// Serializes the cache/depotcache commit shared by the periodic worker and
/// explicit Workshop actions. Hubcap request deduplication alone is not enough:
/// two callers could otherwise race on the same temporary cache filename.
fn workshop_commit_gate() -> &'static Mutex<()> {
    static GATE: OnceLock<Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(()))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err(
            "Workshop manifest cache cannot be committed because the payload is empty".to_string(),
        );
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Workshop manifest cache has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create Workshop manifest cache: {e}"))?;
    let temporary = path.with_extension("manifest.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not write Workshop manifest cache: {e}"))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("Could not commit Workshop manifest cache: {e}"))?;
    let cached_len = std::fs::metadata(path)
        .map_err(|e| format!("Could not verify Workshop manifest cache: {e}"))?
        .len();
    if cached_len != bytes.len() as u64 {
        return Err(format!(
            "Workshop manifest cache is incomplete (expected {} bytes, found {})",
            bytes.len(),
            cached_len
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkshopItem {
    pub(crate) app_id: u32,
    pub(crate) workshop_id: u64,
    pub(crate) manifest_gid: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkshopTarget {
    /// The manifest Aether must stage. For an installed item this may be a
    /// newer `latest_manifest`; for a subscription it is the details manifest.
    pub(crate) item: WorkshopItem,
    /// The manifest Steam currently has installed, when this target came from
    /// `WorkshopItemsInstalled`. A different target means an update.
    pub(crate) installed_manifest_gid: Option<u64>,
    /// The target came only from `WorkshopItemDetails`, so Steam has not
    /// installed it yet. The background monitor never processes these.
    pub(crate) subscription: bool,
    /// `appworkshop_<app_id>.acf` has `NeedsDownload "1"`.
    pub(crate) needs_download: bool,
}

impl WorkshopTarget {
    pub(crate) fn is_update(&self) -> bool {
        self.installed_manifest_gid
            .is_some_and(|gid| gid != self.item.manifest_gid)
    }
}

struct WorkshopSelection {
    targets: Vec<WorkshopTarget>,
    installed: usize,
    subscriptions: usize,
}

/// The background monitor passes `false` and never spends Workshop quota on
/// subscriptions; the explicit Repair action passes `true`.
fn select_workshop_targets(
    discovered: Vec<WorkshopTarget>,
    include_subscriptions: bool,
) -> WorkshopSelection {
    let mut installed = Vec::new();
    let mut subscriptions = Vec::new();
    for target in discovered {
        if target.subscription {
            subscriptions.push(target);
        } else {
            installed.push(target);
        }
    }
    let selected_subscriptions = if include_subscriptions {
        subscriptions.len()
    } else {
        0
    };
    let installed_count = installed.len();
    // The explicit repair prioritizes new subscriptions: the background
    // monitor already retries installed updates, while a subscription waits
    // for the next manual click. Both still share the per-run generation cap.
    let targets = if include_subscriptions {
        subscriptions.into_iter().chain(installed).collect()
    } else {
        installed
    };
    WorkshopSelection {
        targets,
        installed: installed_count,
        subscriptions: selected_subscriptions,
    }
}

/// Scans every library's `appworkshop_*.acf` and returns every repair target:
/// installed items (including updates) and `WorkshopItemDetails` subscriptions.
///
/// The per-branch `manifest_history` blocks use small numeric keys that look
/// like Workshop IDs; the scoped parser ignores them. A subscription without a
/// positive `manifest` is not a target: `latest_manifest` alone is not enough
/// to stage a manifest Steam will accept. The caller decides whether to include
/// subscription targets; the background monitor does not.
pub(crate) fn discover_workshop_items(steam_path: &str) -> Vec<WorkshopTarget> {
    let libraries =
        crate::steam::library::SteamLibraryScanner::new(steam_path).discover_library_paths();
    let mut installed = Vec::new();
    let mut subscriptions = Vec::new();
    let mut legacy_total = 0usize;

    for library in &libraries {
        let workshop_dir = library.join("steamapps").join("workshop");
        let Ok(entries) = std::fs::read_dir(workshop_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_workshop_acf = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("appworkshop_") && name.ends_with(".acf"))
                .unwrap_or(false);
            if !is_workshop_acf {
                continue;
            }
            let Some(app_id) = path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("appworkshop_"))
                .and_then(|id| id.parse::<u32>().ok())
                .filter(|id| *id > 0)
            else {
                continue;
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(content) => content,
                Err(error) => {
                    crate::desk_log_warn!(
                        "workshop",
                        "Skipping unreadable Workshop ACF {}: {}",
                        path.display(),
                        error
                    );
                    continue;
                }
            };
            let parsed = match crate::steam::appworkshop::parse_workshop_acf(&content) {
                Ok(parsed) => parsed,
                Err(error) => {
                    crate::desk_log_warn!(
                        "workshop",
                        "Skipping malformed Workshop ACF {}: {}",
                        path.display(),
                        error
                    );
                    continue;
                }
            };
            legacy_total += parsed.legacy_installed;
            for item in parsed.installed {
                let target_gid = if should_use_latest(&item, parsed.needs_download) {
                    item.latest_manifest_gid.unwrap_or(item.manifest_gid)
                } else {
                    item.manifest_gid
                };
                installed.push(WorkshopTarget {
                    item: WorkshopItem {
                        app_id,
                        workshop_id: item.workshop_id,
                        manifest_gid: target_gid,
                    },
                    installed_manifest_gid: Some(item.manifest_gid),
                    subscription: false,
                    needs_download: parsed.needs_download,
                });
            }
            for item in parsed.subscribed {
                subscriptions.push(WorkshopTarget {
                    item: WorkshopItem {
                        app_id,
                        workshop_id: item.workshop_id,
                        manifest_gid: item.manifest_gid,
                    },
                    installed_manifest_gid: None,
                    subscription: true,
                    needs_download: parsed.needs_download,
                });
            }
        }
    }

    installed.sort_by_key(|target| (target.item.workshop_id, target.item.app_id));
    installed.dedup_by_key(|target| target.item.workshop_id);

    let installed_ids: std::collections::HashSet<u64> = installed
        .iter()
        .map(|target| target.item.workshop_id)
        .collect();
    subscriptions.sort_by_key(|target| (target.item.workshop_id, target.item.app_id));
    subscriptions.dedup_by_key(|target| target.item.workshop_id);
    subscriptions.retain(|target| !installed_ids.contains(&target.item.workshop_id));

    crate::desk_log_debug!(
        "workshop",
        "Workshop discovery installed={} subscriptions={} legacy_skipped={}",
        installed.len(),
        subscriptions.len(),
        legacy_total
    );
    installed.extend(subscriptions);
    installed
}

/// `latest_manifest` wins only when Steam says it is newer than the installed
/// manifest. Some ACFs omit the timestamps; in that case `NeedsDownload` is the
/// only signal that Steam is waiting for a new manifest.
fn should_use_latest(
    item: &crate::steam::appworkshop::WorkshopAcfItem,
    needs_download: bool,
) -> bool {
    let Some(latest) = item.latest_manifest_gid else {
        return false;
    };
    if latest == item.manifest_gid {
        return false;
    }
    match (item.time_updated, item.latest_time_updated) {
        (Some(installed), Some(latest_time)) => latest_time > installed,
        _ => needs_download,
    }
}

/// Steam manifests are four little-endian framed protobuf sections; the
/// shared reader in `manifest::identity` extracts fields 1 (depot_id) and 2
/// (gid_manifest) of the metadata section and unwraps a single-entry ZIP
/// wrapper when Hubcap returns one.
fn manifest_identity(bytes: &[u8]) -> Result<(u32, u64), String> {
    crate::manifest::identity::manifest_identity(bytes)
        .map_err(|error| format!("Workshop manifest: {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncOrigin {
    Generated,
    Cached,
    AlreadyLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SyncOutcome {
    origin: SyncOrigin,
    content_present: bool,
    content_empty: bool,
    update: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ContentState {
    present: bool,
    empty: bool,
}

fn has_local_manifest_gid(steam_path: &str, manifest_gid: u64) -> bool {
    let file_name_suffix = format!("_{manifest_gid}.manifest");
    [
        PathBuf::from(steam_path).join("depotcache"),
        PathBuf::from(steam_path).join("config").join("depotcache"),
    ]
    .into_iter()
    .filter_map(|directory| std::fs::read_dir(directory).ok())
    .flat_map(|entries| entries.flatten())
    .map(|entry| entry.path())
    .any(|path| {
        path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(&file_name_suffix))
                .unwrap_or(false)
            && std::fs::metadata(path)
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false)
    })
}

/// A staged `.manifest` is not proof that Steam has materialized the Workshop
/// payload. An empty content directory is not content either: Steam can leave
/// one behind after a cancelled or failed download.
fn workshop_content_state(steam_path: &str, app_id: u32, workshop_id: u64) -> ContentState {
    let mut empty = false;
    for library in
        crate::steam::library::SteamLibraryScanner::new(steam_path).discover_library_paths()
    {
        let path = library
            .join("steamapps")
            .join("workshop")
            .join("content")
            .join(app_id.to_string())
            .join(workshop_id.to_string());
        if !path.is_dir() {
            continue;
        }
        match std::fs::read_dir(&path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return ContentState {
                        present: true,
                        empty: false,
                    };
                }
                empty = true;
            }
            Err(_) => continue,
        }
    }
    ContentState {
        present: false,
        empty,
    }
}

/// True when neither depotcache nor the validated Workshop cache has the
/// expected manifest, so only Hubcap can supply it.
fn needs_remote_manifest(steam_path: &str, item: WorkshopItem) -> bool {
    !has_local_manifest_gid(steam_path, item.manifest_gid)
        && !cached_workshop_manifest_matches(item.workshop_id, item.manifest_gid)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkshopItemAction {
    Run { needs_remote: bool },
    SkipUnavailable,
    DeferGeneration,
}

/// The per-item decision, kept pure so the interaction between local/cache
/// hits, the terminal-404 cache and the per-run generation cap is testable.
fn decide_item_action(
    needs_remote: bool,
    unavailable: bool,
    remote_attempts: usize,
) -> WorkshopItemAction {
    if needs_remote && unavailable {
        return WorkshopItemAction::SkipUnavailable;
    }
    if needs_remote && remote_attempts >= WORKSHOP_MAX_GENERATIONS_PER_RUN {
        return WorkshopItemAction::DeferGeneration;
    }
    WorkshopItemAction::Run { needs_remote }
}

fn failure_reason(error: &str) -> (Option<u16>, WorkshopItemReason) {
    let http_status = crate::core::workshop_failures::http_status_from_error(error);
    let reason = if crate::core::workshop_failures::is_hubcap_not_found(error) {
        WorkshopItemReason::HubcapNotFound
    } else {
        WorkshopItemReason::UnknownError
    };
    (http_status, reason)
}

fn make_report_item(
    target: WorkshopTarget,
    app_name: Option<&String>,
    status: WorkshopItemStatus,
    reason: Option<WorkshopItemReason>,
    http_status: Option<u16>,
) -> WorkshopItemReport {
    WorkshopItemReport {
        app_id: target.item.app_id,
        app_name: app_name.cloned(),
        workshop_id: target.item.workshop_id,
        title: None,
        manifest_gid: target.item.manifest_gid.to_string(),
        status,
        reason,
        http_status,
        content_empty: false,
        needs_download: target.needs_download,
        subscription: target.subscription,
        update: target.is_update(),
    }
}

fn report_item_for_outcome(
    target: WorkshopTarget,
    app_name: Option<&String>,
    outcome: SyncOutcome,
) -> Option<WorkshopItemReport> {
    if outcome.content_present && !outcome.update {
        return None;
    }
    let mut report_item = make_report_item(target, app_name, WorkshopItemStatus::Ready, None, None);
    report_item.content_empty = outcome.content_empty;
    Some(report_item)
}

fn record_sync_outcome(
    report: &mut WorkshopSyncReport,
    target: WorkshopTarget,
    app_name: Option<&String>,
    outcome: SyncOutcome,
) {
    match outcome.origin {
        SyncOrigin::Generated => report.generated += 1,
        SyncOrigin::Cached => report.restored_from_cache += 1,
        SyncOrigin::AlreadyLocal => {
            if outcome.content_present && !outcome.update {
                report.already_local += 1;
            }
        }
    }
    if outcome.update {
        report.updates += 1;
    }
    if !outcome.content_present {
        report.content_missing += 1;
    }
    if let Some(report_item) = report_item_for_outcome(target, app_name, outcome) {
        report.items.push(report_item);
    }
}

fn apply_titles(report: &mut WorkshopSyncReport, titles: std::collections::HashMap<u64, String>) {
    for item in &mut report.items {
        item.title = titles.get(&item.workshop_id).cloned();
    }
}

async fn enrich_report_titles(report: &mut WorkshopSyncReport) {
    if report.items.is_empty() {
        return;
    }
    let workshop_ids = report.items.iter().map(|item| item.workshop_id).collect();
    match crate::steam::workshop_titles::fetch_titles(workshop_ids).await {
        Ok(titles) => apply_titles(report, titles),
        Err(error) => crate::desk_log_warn!("workshop", "Workshop title lookup failed: {}", error),
    }
}

/// Per-run cap on remote Hubcap generations for the periodic synchronizer.
/// Local/cache hits are unlimited: only provider traffic is rationed. The
/// monitor re-queues the lane when items are deferred, so the backlog drains
/// in bounded batches instead of one startup storm.
const WORKSHOP_MAX_GENERATIONS_PER_RUN: usize = 5;

async fn sync_one_workshop_item(
    client: HubcapClient,
    steam_path: String,
    target: WorkshopTarget,
) -> Result<SyncOutcome, String> {
    let item = target.item;
    let started = Instant::now();
    crate::desk_log_info!(
        "workshop",
        "Sync start workshop_id={} expected_manifest_gid={} update={}",
        item.workshop_id,
        item.manifest_gid,
        target.is_update()
    );
    if steam_path.trim().is_empty() {
        return Err(
            "Workshop sync cannot install manifests because the Steam path is empty".to_string(),
        );
    }
    let _commit_guard = workshop_commit_gate().lock().await;
    if has_local_manifest_gid(&steam_path, item.manifest_gid) {
        let content = workshop_content_state(&steam_path, item.app_id, item.workshop_id);
        crate::desk_log_debug!(
            "workshop",
            "Sync manifest local workshop_id={} manifest_gid={} content_present={} content_empty={}",
            item.workshop_id,
            item.manifest_gid,
            content.present,
            content.empty
        );
        return Ok(SyncOutcome {
            origin: SyncOrigin::AlreadyLocal,
            content_present: content.present,
            content_empty: content.empty,
            update: target.is_update(),
        });
    }
    let cache_path = workshop_cache_path(item.workshop_id);
    if cache_path.is_file() {
        match std::fs::read(&cache_path) {
            Ok(bytes) => match manifest_identity(&bytes) {
                Ok((depot_id, gid)) if gid == item.manifest_gid => {
                    crate::desk_log_info!(
                        "workshop",
                        "Sync using validated cache workshop_id={} bytes={} depot_id={} manifest_gid={}",
                        item.workshop_id,
                        bytes.len(),
                        depot_id,
                        gid
                    );
                    install_to_depotcache(&steam_path, depot_id, gid, &bytes)?;
                    backup_workshop_manifest(item.app_id, depot_id, gid, &bytes);
                    let content =
                        workshop_content_state(&steam_path, item.app_id, item.workshop_id);
                    crate::desk_log_info!(
                        "workshop",
                        "Sync complete origin=cache workshop_id={} content_present={} content_empty={} elapsed_ms={}",
                        item.workshop_id,
                        content.present,
                        content.empty,
                        started.elapsed().as_millis()
                    );
                    return Ok(SyncOutcome {
                        origin: SyncOrigin::Cached,
                        content_present: content.present,
                        content_empty: content.empty,
                        update: target.is_update(),
                    });
                }
                Ok((_depot_id, gid)) => crate::desk_log_warn!(
                    "workshop",
                    "Ignoring stale Workshop cache workshop_id={} expected_manifest_gid={} cached_manifest_gid={}",
                    item.workshop_id,
                    item.manifest_gid,
                    gid
                ),
                Err(error) => crate::desk_log_warn!(
                    "workshop",
                    "Ignoring invalid Workshop cache workshop_id={}: {}",
                    item.workshop_id,
                    error
                ),
            },
            Err(error) => crate::desk_log_warn!(
                "workshop",
                "Workshop cache read failed workshop_id={} path={}: {}",
                item.workshop_id,
                cache_path.display(),
                error
            ),
        }
    }
    crate::desk_log_info!(
        "workshop",
        "Sync requesting authenticated Hubcap Workshop manifest workshop_id={}",
        item.workshop_id
    );
    let bytes = client.generate_workshop_manifest(item.workshop_id).await?;
    let (depot_id, gid) = manifest_identity(&bytes)?;
    crate::desk_log_debug!(
        "workshop",
        "Hubcap Workshop manifest validated workshop_id={} bytes={} depot_id={} manifest_gid={}",
        item.workshop_id,
        bytes.len(),
        depot_id,
        gid
    );
    if gid != item.manifest_gid {
        return Err(format!(
            "Hubcap Workshop manifest mismatch for item {} (expected {}, received {})",
            item.workshop_id, item.manifest_gid, gid
        ));
    }
    write_atomic(&cache_path, &bytes)?;
    install_to_depotcache(&steam_path, depot_id, gid, &bytes)?;
    backup_workshop_manifest(item.app_id, depot_id, gid, &bytes);
    let content = workshop_content_state(&steam_path, item.app_id, item.workshop_id);
    crate::desk_log_info!(
        "workshop",
        "Sync complete origin=generated workshop_id={} bytes={} content_present={} content_empty={} elapsed_ms={}",
        item.workshop_id,
        bytes.len(),
        content.present,
        content.empty,
        started.elapsed().as_millis()
    );
    Ok(SyncOutcome {
        origin: SyncOrigin::Generated,
        content_present: content.present,
        content_empty: content.empty,
        update: target.is_update(),
    })
}

/// Mirrors a freshly staged Workshop manifest into the per-game AetherData
/// backup (`backup/<app_id>/lua/<depot>_<gid>.manifest`) so the standard
/// uninstall/restore contract covers Workshop manifests too. Neither the DLL
/// startup backup nor `backup_referenced_manifests` can capture them: no Lua
/// pin references a Workshop depot. The Hubcap Workshop cache is keyed by
/// item id, while the backup tree keeps the manifest identity. Best-effort.
fn backup_workshop_manifest(app_id: u32, depot_id: u32, manifest_gid: u64, bytes: &[u8]) {
    let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) else {
        return;
    };
    if backup.backup_manifest_bytes(&format!("{depot_id}_{manifest_gid}.manifest"), bytes) {
        crate::desk_log_info!(
            "workshop",
            "Workshop manifest archived in AetherData backup app_id={} depot_id={} manifest_gid={} bytes={}",
            app_id,
            depot_id,
            manifest_gid,
            bytes.len()
        );
    }
}

fn install_to_depotcache(
    steam_path: &str,
    depot_id: u32,
    manifest_gid: u64,
    bytes: &[u8],
) -> Result<(), String> {
    if bytes.is_empty() {
        return Err(
            "Workshop manifest cannot be installed because the payload is empty".to_string(),
        );
    }
    let directory = PathBuf::from(steam_path).join("depotcache");
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("Could not create Steam depotcache for Workshop: {e}"))?;
    let path = directory.join(format!("{depot_id}_{manifest_gid}.manifest"));
    if path.is_file() {
        let existing_len = std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if existing_len > 0 && existing_len == bytes.len() as u64 {
            crate::desk_log_debug!(
                "workshop",
                "Install verified existing depotcache manifest depot_id={} manifest_gid={} bytes={}",
                depot_id,
                manifest_gid,
                existing_len
            );
            return Ok(());
        }
        crate::desk_log_warn!(
            "workshop",
            "Replacing incomplete or stale depotcache manifest depot_id={} manifest_gid={} existing_bytes={} expected_bytes={}",
            depot_id,
            manifest_gid,
            existing_len,
            bytes.len()
        );
    }
    let temporary = path.with_extension("manifest.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not stage Workshop manifest: {e}"))?;
    std::fs::rename(&temporary, &path)
        .map_err(|e| format!("Could not install Workshop manifest: {e}"))?;
    let installed_len = std::fs::metadata(&path)
        .map_err(|e| format!("Could not verify installed Workshop manifest: {e}"))?
        .len();
    if installed_len != bytes.len() as u64 || installed_len == 0 {
        return Err(format!(
            "Installed Workshop manifest is incomplete (expected {} bytes, found {})",
            bytes.len(),
            installed_len
        ));
    }
    crate::desk_log_info!(
        "workshop",
        "Workshop manifest installed path={} bytes={} depot_id={} manifest_gid={}",
        path.display(),
        installed_len,
        depot_id,
        manifest_gid
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkshopItemStatus {
    Ready,
    Deferred,
    Failed,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkshopItemReason {
    HubcapNotFound,
    UnknownError,
    GenerationCap,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopItemReport {
    pub app_id: u32,
    pub app_name: Option<String>,
    pub workshop_id: u64,
    /// Workshop item title, when Steam's public metadata lookup succeeded.
    pub title: Option<String>,
    /// String on purpose: Steam manifest GIDs exceed JavaScript's safe integer.
    pub manifest_gid: String,
    pub status: WorkshopItemStatus,
    /// `None` for a ready item: being ready is not a failure.
    pub reason: Option<WorkshopItemReason>,
    pub http_status: Option<u16>,
    /// The content directory existed but had no entries.
    pub content_empty: bool,
    /// Steam's ACF says it has a pending Workshop download.
    pub needs_download: bool,
    /// The item came only from `WorkshopItemDetails`.
    pub subscription: bool,
    /// The target is a newer manifest than `WorkshopItemsInstalled.manifest`.
    pub update: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopSyncReport {
    /// Installed Workshop items considered in this run.
    pub discovered: usize,
    /// `WorkshopItemDetails` subscriptions considered in this run.
    pub subscriptions: usize,
    pub generated: usize,
    pub restored_from_cache: usize,
    pub already_local: usize,
    /// Items with a valid staged manifest but without Steam's actual Workshop
    /// content directory. They are intentionally not counted as complete.
    pub content_missing: usize,
    /// Installed items whose target manifest is newer than the installed one.
    pub updates: usize,
    pub failed: usize,
    /// Items pushed to the next lane pass by the per-run generation cap.
    /// NOT failures: the monitor re-runs the lane shortly, so a depotcache wipe
    /// can never burn the whole daily quota in one storm.
    pub deferred: usize,
    /// Items skipped by the 24 h terminal-404 cache.
    pub unavailable: usize,
    pub items: Vec<WorkshopItemReport>,
}

/// Scans Steam's appworkshop ACF files and proactively stages missing
/// Workshop manifests through authenticated Hubcap. The coordinator invokes it
/// only after an ACF fingerprint change; exact local files and validated caches
/// are skipped before any provider validation or generation request.
///
/// Installed items include updates whose `latest_manifest` is newer than the
/// installed one. `include_subscriptions` is true only for the explicit Repair
/// action: the background monitor must not spend Workshop quota on
/// `WorkshopItemDetails`-only subscriptions.
#[tauri::command]
pub async fn sync_hubcap_workshop_manifests(
    app: tauri::AppHandle,
    include_subscriptions: bool,
) -> Result<WorkshopSyncReport, String> {
    let started = Instant::now();
    let settings = load_settings(&app);
    crate::desk_log_info!(
        "workshop",
        "Workshop sync start key_configured={} steam_path_configured={} include_subscriptions={}",
        !settings.hubcap_api_key.trim().is_empty(),
        !settings.steam_path.trim().is_empty(),
        include_subscriptions
    );
    if settings.steam_path.trim().is_empty() {
        crate::desk_log_error!(
            "workshop",
            "Workshop sync cannot run because the Steam path is empty"
        );
        return Err("Workshop sync cannot run because the Steam path is empty".to_string());
    }
    let steam_path = settings.steam_path;
    let selection =
        select_workshop_targets(discover_workshop_items(&steam_path), include_subscriptions);
    crate::desk_log_info!(
        "workshop",
        "Workshop discovery complete installed={} subscriptions={}",
        selection.installed,
        selection.subscriptions
    );
    let mut report = WorkshopSyncReport {
        discovered: selection.installed,
        subscriptions: selection.subscriptions,
        ..Default::default()
    };
    let targets = selection.targets;
    if targets.is_empty() {
        crate::desk_log_debug!(
            "workshop",
            "Workshop sync complete no installed or included subscription Workshop items"
        );
        return Ok(report);
    }

    let app_cache_dir = LocalAppPaths::data_root().join("cache");
    let app_ids: Vec<u32> = targets.iter().map(|target| target.item.app_id).collect();
    let app_names =
        crate::steam::app_names::SteamAppNameResolver::new(&app_cache_dir).cached_names(app_ids);

    let now = crate::core::workshop_failures::now_unix();
    let mut failures = crate::core::workshop_failures::WorkshopFailureCache::load();
    failures.prune(now);

    let mut runnable = Vec::with_capacity(targets.len());
    for target in targets {
        // A local or cached manifest wins over a remembered 404: the cache only
        // suppresses a provider request, it never hides a payload Steam can
        // already use.
        let needs_remote = needs_remote_manifest(&steam_path, target.item);
        if needs_remote
            && failures.is_unavailable(target.item.workshop_id, target.item.manifest_gid, now)
        {
            report.unavailable += 1;
            report.items.push(make_report_item(
                target,
                app_names.get(&target.item.app_id),
                WorkshopItemStatus::Unavailable,
                Some(WorkshopItemReason::HubcapNotFound),
                Some(404),
            ));
        } else {
            runnable.push(target);
        }
    }

    let needs_remote_generation = runnable
        .iter()
        .any(|target| needs_remote_manifest(&steam_path, target.item));
    crate::desk_log_debug!(
        "workshop",
        "Workshop sync local/cache gate runnable={} unavailable={} needs_remote_generation={}",
        runnable.len(),
        report.unavailable,
        needs_remote_generation
    );
    if needs_remote_generation && settings.hubcap_api_key.trim().is_empty() {
        return Err(
            "A valid authenticated Hubcap API key is required for missing Workshop manifests."
                .to_string(),
        );
    }
    let client = HubcapClient::new(&settings.hubcap_api_key);
    if needs_remote_generation && !client.validate_api_key().await? {
        crate::desk_log_error!(
            "workshop",
            "Workshop sync stopped because the Hubcap API key is not valid or not allowed"
        );
        return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
    }
    // The provider rate-limits generation separately from the daily quota, and
    // the process-wide scheduler already serializes generation traffic shared
    // with store/versioning actions: process the items one at a time. Remote
    // generations are additionally capped per run; deferred items are picked
    // up by the next lane pass.
    let mut remote_attempts = 0usize;

    for target in runnable {
        match decide_item_action(
            needs_remote_manifest(&steam_path, target.item),
            failures.is_unavailable(target.item.workshop_id, target.item.manifest_gid, now),
            remote_attempts,
        ) {
            WorkshopItemAction::SkipUnavailable => {
                report.unavailable += 1;
                report.items.push(make_report_item(
                    target,
                    app_names.get(&target.item.app_id),
                    WorkshopItemStatus::Unavailable,
                    Some(WorkshopItemReason::HubcapNotFound),
                    Some(404),
                ));
                continue;
            }
            WorkshopItemAction::DeferGeneration => {
                report.deferred += 1;
                crate::desk_log_info!(
                    "workshop",
                    "Workshop target deferred workshop_id={} reason=generation_cap",
                    target.item.workshop_id
                );
                report.items.push(make_report_item(
                    target,
                    app_names.get(&target.item.app_id),
                    WorkshopItemStatus::Deferred,
                    Some(WorkshopItemReason::GenerationCap),
                    None,
                ));
                continue;
            }
            WorkshopItemAction::Run { needs_remote } => {
                if needs_remote {
                    remote_attempts += 1;
                }
            }
        }
        match sync_one_workshop_item(client.clone(), steam_path.clone(), target).await {
            Ok(outcome) => {
                record_sync_outcome(
                    &mut report,
                    target,
                    app_names.get(&target.item.app_id),
                    outcome,
                );
            }
            Err(error) => {
                report.failed += 1;
                let (http_status, reason) = failure_reason(&error);
                if http_status == Some(404) {
                    failures.record(target.item.workshop_id, target.item.manifest_gid, now);
                }
                report.items.push(make_report_item(
                    target,
                    app_names.get(&target.item.app_id),
                    WorkshopItemStatus::Failed,
                    Some(reason),
                    http_status,
                ));
                crate::desk_log_warn!("hubcap", "Workshop manifest preparation failed: {}", error);
            }
        }
    }

    if include_subscriptions {
        enrich_report_titles(&mut report).await;
    }

    if let Err(error) = failures.save() {
        crate::desk_log_warn!(
            "workshop",
            "Could not persist Workshop failure cache: {}",
            error
        );
    }

    crate::desk_log_info!(
        "workshop",
        "Workshop sync complete discovered={} subscriptions={} generated={} cached={} already_local={} content_missing={} updates={} failed={} deferred={} unavailable={} elapsed_ms={}",
        report.discovered,
        report.subscriptions,
        report.generated,
        report.restored_from_cache,
        report.already_local,
        report.content_missing,
        report.updates,
        report.failed,
        report.deferred,
        report.unavailable,
        started.elapsed().as_millis()
    );
    Ok(report)
}

/// Explicit single-item entry point used by future Workshop UI flows. It uses
/// the same cache and generation scheduler as the periodic synchronizer.
#[tauri::command]
pub async fn generate_hubcap_workshop_manifest(
    app: tauri::AppHandle,
    workshop_id: u64,
) -> Result<String, String> {
    let started = Instant::now();
    let _commit_guard = workshop_commit_gate().lock().await;
    if workshop_id == 0 {
        return Err("A valid Workshop item ID is required".to_string());
    }
    let path = workshop_cache_path(workshop_id);
    let settings = load_settings(&app);
    crate::desk_log_info!(
        "workshop",
        "Explicit Workshop generation start workshop_id={} key_configured={} steam_path_configured={}",
        workshop_id,
        !settings.hubcap_api_key.trim().is_empty(),
        !settings.steam_path.trim().is_empty()
    );
    if settings.steam_path.trim().is_empty() {
        return Err(
            "Workshop generation cannot complete because the Steam path is empty".to_string(),
        );
    }
    if path.is_file()
        && std::fs::metadata(&path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    {
        match std::fs::read(&path) {
            Ok(bytes) => match manifest_identity(&bytes) {
                Ok((depot_id, manifest_gid)) => {
                    install_to_depotcache(&settings.steam_path, depot_id, manifest_gid, &bytes)?;
                    crate::desk_log_info!(
                        "workshop",
                        "Explicit Workshop generation restored validated cache workshop_id={} path={} elapsed_ms={}",
                        workshop_id,
                        path.display(),
                        started.elapsed().as_millis()
                    );
                    return Ok(path.display().to_string());
                }
                Err(error) => crate::desk_log_warn!(
                    "workshop",
                    "Ignoring invalid explicit Workshop cache workshop_id={}: {}",
                    workshop_id,
                    error
                ),
            },
            Err(error) => crate::desk_log_warn!(
                "workshop",
                "Explicit Workshop cache read failed workshop_id={}: {}",
                workshop_id,
                error
            ),
        }
    }
    if settings.hubcap_api_key.trim().is_empty() {
        return Err(
            "A valid Hubcap API key is required for Workshop manifest generation.".to_string(),
        );
    }
    let hubcap = HubcapClient::new(&settings.hubcap_api_key);
    if !hubcap.validate_api_key().await? {
        return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
    }
    let bytes = hubcap.generate_workshop_manifest(workshop_id).await?;
    // Validate before caching so a provider error or wrong content type can
    // never become a persistent fake Workshop manifest.
    let (depot_id, manifest_gid) = manifest_identity(&bytes)?;
    write_atomic(&path, &bytes)?;
    install_to_depotcache(&settings.steam_path, depot_id, manifest_gid, &bytes)?;
    crate::desk_log_info!(
        "workshop",
        "Explicit Workshop generation complete workshop_id={} bytes={} depot_id={} manifest_gid={} path={} elapsed_ms={}",
        workshop_id,
        bytes.len(),
        depot_id,
        manifest_gid,
        path.display(),
        started.elapsed().as_millis()
    );
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn workshop_target() -> WorkshopTarget {
        WorkshopTarget {
            item: WorkshopItem {
                app_id: 3328490,
                workshop_id: 3806068832,
                manifest_gid: 3748992252827399921,
            },
            installed_manifest_gid: Some(3748992252827399921),
            subscription: false,
            needs_download: false,
        }
    }

    fn subscription_target() -> WorkshopTarget {
        WorkshopTarget {
            installed_manifest_gid: None,
            subscription: true,
            ..workshop_target()
        }
    }

    fn outcome(content_present: bool) -> SyncOutcome {
        SyncOutcome {
            origin: SyncOrigin::Generated,
            content_present,
            content_empty: false,
            update: false,
        }
    }

    #[test]
    fn report_marks_content_missing_as_ready() {
        let report = report_item_for_outcome(workshop_target(), None, outcome(false)).unwrap();

        assert_eq!(report.status, WorkshopItemStatus::Ready);
        assert_eq!(report.reason, None);
        assert_eq!(report.workshop_id, 3806068832);
    }

    #[test]
    fn report_marks_generation_cap_as_deferred() {
        let report = make_report_item(
            workshop_target(),
            None,
            WorkshopItemStatus::Deferred,
            Some(WorkshopItemReason::GenerationCap),
            None,
        );

        assert_eq!(report.status, WorkshopItemStatus::Deferred);
        assert_eq!(report.reason, Some(WorkshopItemReason::GenerationCap));
    }

    #[test]
    fn report_lists_an_update_even_when_content_is_present() {
        let target = WorkshopTarget {
            item: WorkshopItem {
                manifest_gid: 900,
                ..workshop_target().item
            },
            installed_manifest_gid: Some(3748992252827399921),
            ..workshop_target()
        };
        let update = SyncOutcome {
            content_present: true,
            update: true,
            ..outcome(true)
        };

        let report = report_item_for_outcome(target, None, update).unwrap();

        assert_eq!(report.status, WorkshopItemStatus::Ready);
        assert!(report.update);
        assert!(!report.content_empty);
    }

    #[test]
    fn report_marks_subscription_targets() {
        let report = report_item_for_outcome(subscription_target(), None, outcome(false)).unwrap();

        assert!(report.subscription);
        assert!(!report.update);
    }

    #[test]
    fn report_classifies_404_as_hubcap_not_found() {
        let (status, reason) =
            failure_reason("Hubcap Workshop manifest request http_error (HTTP 404)");

        assert_eq!(status, Some(404));
        assert_eq!(reason, WorkshopItemReason::HubcapNotFound);
    }

    #[test]
    fn report_classifies_other_errors_as_unknown() {
        let (status, reason) =
            failure_reason("Hubcap Workshop manifest request provider_unavailable (HTTP 503)");

        assert_eq!(status, Some(503));
        assert_eq!(reason, WorkshopItemReason::UnknownError);

        let (status, reason) = failure_reason("network timeout");
        assert_eq!(status, None);
        assert_eq!(reason, WorkshopItemReason::UnknownError);
    }

    #[test]
    fn report_does_not_list_already_local_items() {
        assert!(report_item_for_outcome(workshop_target(), None, outcome(true)).is_none());
    }

    #[test]
    fn workshop_content_state_treats_empty_directory_as_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let content = root
            .join("steamapps")
            .join("workshop")
            .join("content")
            .join("3328490")
            .join("3806068832");
        fs::create_dir_all(&content).unwrap();

        assert_eq!(
            workshop_content_state(root.to_str().unwrap(), 3328490, 3806068832),
            ContentState {
                present: false,
                empty: true,
            }
        );

        fs::write(content.join("payload.bin"), b"payload").unwrap();
        assert_eq!(
            workshop_content_state(root.to_str().unwrap(), 3328490, 3806068832),
            ContentState {
                present: true,
                empty: false,
            }
        );
    }
}

#[cfg(test)]
mod review_fix_tests {
    use super::*;
    use std::fs;

    const TARGET: WorkshopTarget = WorkshopTarget {
        item: WorkshopItem {
            app_id: 3328490,
            workshop_id: 3806068832,
            manifest_gid: 3748992252827399921,
        },
        installed_manifest_gid: Some(3748992252827399921),
        subscription: false,
        needs_download: false,
    };

    const SUBSCRIPTION_TARGET: WorkshopTarget = WorkshopTarget {
        installed_manifest_gid: None,
        subscription: true,
        ..TARGET
    };

    fn outcome(content_present: bool) -> SyncOutcome {
        SyncOutcome {
            origin: SyncOrigin::Generated,
            content_present,
            content_empty: false,
            update: false,
        }
    }

    #[test]
    fn monitor_selection_excludes_subscription_targets() {
        let selection = select_workshop_targets(vec![TARGET, SUBSCRIPTION_TARGET], false);

        assert_eq!(selection.installed, 1);
        assert_eq!(selection.subscriptions, 0);
        assert_eq!(selection.targets.len(), 1);
        assert!(!selection.targets[0].subscription);
    }

    #[test]
    fn manual_repair_selection_includes_subscription_targets() {
        let selection = select_workshop_targets(vec![TARGET, SUBSCRIPTION_TARGET], true);

        assert_eq!(selection.installed, 1);
        assert_eq!(selection.subscriptions, 1);
        assert_eq!(selection.targets.len(), 2);
        assert!(selection.targets[0].subscription);
        assert!(!selection.targets[1].subscription);
    }

    #[test]
    fn decide_item_action_runs_local_items_even_when_a_404_is_cached() {
        assert_eq!(
            decide_item_action(false, true, 0),
            WorkshopItemAction::Run {
                needs_remote: false
            }
        );
    }

    #[test]
    fn decide_item_action_skips_remote_items_with_a_cached_404() {
        assert_eq!(
            decide_item_action(true, true, 0),
            WorkshopItemAction::SkipUnavailable
        );
    }

    #[test]
    fn decide_item_action_defers_only_remote_items_at_the_cap() {
        assert_eq!(
            decide_item_action(true, false, WORKSHOP_MAX_GENERATIONS_PER_RUN),
            WorkshopItemAction::DeferGeneration
        );
        assert_eq!(
            decide_item_action(false, false, WORKSHOP_MAX_GENERATIONS_PER_RUN),
            WorkshopItemAction::Run {
                needs_remote: false
            }
        );
    }

    #[test]
    fn needs_remote_manifest_sees_a_local_depotcache_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let depotcache = root.join("depotcache");
        fs::create_dir_all(&depotcache).unwrap();
        fs::write(depotcache.join("123_456.manifest"), b"manifest").unwrap();

        let local = WorkshopItem {
            app_id: 1,
            workshop_id: 2,
            manifest_gid: 456,
        };
        assert!(!needs_remote_manifest(root.to_str().unwrap(), local));
    }

    #[test]
    fn record_sync_outcome_counts_updates() {
        let mut report = WorkshopSyncReport::default();
        let target = WorkshopTarget {
            item: WorkshopItem {
                manifest_gid: 900,
                ..TARGET.item
            },
            ..TARGET
        };
        let update = SyncOutcome {
            content_present: true,
            update: true,
            ..outcome(true)
        };
        record_sync_outcome(&mut report, target, None, update);

        assert_eq!(report.already_local, 0);
        assert_eq!(report.content_missing, 0);
        assert_eq!(report.updates, 1);
        assert_eq!(report.items.len(), 1);
        assert!(report.items[0].update);
    }

    #[test]
    fn record_sync_outcome_does_not_list_already_local_items() {
        let mut report = WorkshopSyncReport::default();
        let local = SyncOutcome {
            origin: SyncOrigin::AlreadyLocal,
            content_present: true,
            content_empty: false,
            update: false,
        };
        record_sync_outcome(&mut report, TARGET, None, local);

        assert_eq!(report.already_local, 1);
        assert!(report.items.is_empty());
    }

    #[test]
    fn report_serializes_manifest_gid_as_a_string() {
        let report = make_report_item(TARGET, None, WorkshopItemStatus::Ready, None, None);
        let value = serde_json::to_value(&report).unwrap();

        assert_eq!(value["manifestGid"], "3748992252827399921");
        assert_eq!(value["status"], "ready");
        assert_eq!(value["reason"], serde_json::Value::Null);
        assert_eq!(value["title"], serde_json::Value::Null);
    }

    #[test]
    fn apply_titles_matches_items_by_workshop_id() {
        let mut other = TARGET;
        other.item.workshop_id = 222;
        let mut report = WorkshopSyncReport::default();
        report.items.push(make_report_item(
            TARGET,
            None,
            WorkshopItemStatus::Ready,
            None,
            None,
        ));
        report.items.push(make_report_item(
            other,
            None,
            WorkshopItemStatus::Ready,
            None,
            None,
        ));

        apply_titles(
            &mut report,
            std::collections::HashMap::from([(3806068832, "Uniforms of the Realm".to_string())]),
        );

        assert_eq!(
            report.items[0].title.as_deref(),
            Some("Uniforms of the Realm")
        );
        assert_eq!(report.items[1].title, None);
    }
}
