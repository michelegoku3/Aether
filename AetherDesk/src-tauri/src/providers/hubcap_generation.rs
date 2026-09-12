//! Authenticated Hubcap generation scheduling.
//!
//! This module owns the shared quota and deduplication policy. It never logs
//! API keys, never requests a manifest that already exists locally (the
//! caller performs the exact local check), and coalesces identical concurrent
//! generation requests so one user action cannot spend the same quota twice.
//!
//! The daily budget lives in ONE file —
//! `<AetherData>\state\hubcap_generation_quota.json` — shared with AetherDLL,
//! which generates manifests on demand inside Steam. Both processes run every
//! read-modify-write cycle while holding a sibling `.lock` file, so the
//! counters stay consistent regardless of who writes;
//! `AetherDLL/AetherCore/network/HubcapQuota.cpp` is the C++ twin of this
//! protocol. Two budgets exist (game 1500/day, Workshop 500/day) and both
//! reset at fixed midnight EST, matching the provider's reset.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, Notify, Semaphore};

/// Minimum spacing between logical generation operations. The generation
/// endpoint and its usage check are both provider traffic; serializing them
/// without a cadence still produces bursts when a DLC-heavy package finishes.
const GENERATION_MIN_INTERVAL: Duration = Duration::from_millis(750);

pub const MAX_GAME_GENERATIONS_PER_DAY: u32 = 1_500;
pub const MAX_WORKSHOP_GENERATIONS_PER_DAY: u32 = 500;
const EST_OFFSET_SECONDS: i64 = -5 * 60 * 60;

/// Cross-process lock parameters, mirrored by the DLL twin: the critical
/// section is a tiny read-modify-write, so a lock older than a few seconds
/// means its holder died without releasing.
const QUOTA_LOCK_STALE_AFTER: Duration = Duration::from_secs(5);
const QUOTA_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const QUOTA_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenerationKind {
    Game,
    Workshop,
}

/// Local quota accounting for one deduplicated provider operation.
#[derive(Debug, Clone, Copy)]
pub enum QuotaPolicy {
    /// Reserves one unit of the given daily generation budget (the shared
    /// quota file) and releases it when the operation fails.
    Generate(GenerationKind),
    /// No local generation budget: the operation consumes the provider's
    /// daily *manifest* quota, which the caller already checks against
    /// `/user/stats` before downloading. This mirrors the server-side
    /// buckets instead of double-counting package downloads as generations.
    ManifestDownload,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GenerationKey {
    Single { depot_id: u32, manifest_id: u64 },
    AppBundle { app_id: u32, branch: String },
    Workshop { workshop_id: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedQuota {
    /// Calendar date in fixed UTC-5 (EST), never the machine's local timezone.
    day: String,
    game_used: u32,
    workshop_used: u32,
}

#[derive(Debug, Clone, Copy)]
struct QuotaReservation {
    kind: GenerationKind,
}

#[derive(Debug, Clone)]
struct CompletedRequest {
    result: Result<Vec<u8>, String>,
}

#[derive(Default)]
struct InflightSlot {
    result: Mutex<Option<CompletedRequest>>,
    notify: Notify,
}

#[derive(Default)]
struct GenerationState {
    inflight: Mutex<HashMap<GenerationKey, Arc<InflightSlot>>>,
    last_generation_start: Mutex<Option<Instant>>,
}

fn state() -> &'static Arc<GenerationState> {
    static STATE: OnceLock<Arc<GenerationState>> = OnceLock::new();
    STATE.get_or_init(|| Arc::new(GenerationState::default()))
}

/// Hubcap rate-limits the generation endpoints separately from the daily
/// quota. A process-wide gate prevents the Workshop worker, store actions,
/// and versioning from issuing concurrent usage checks/generations that would
/// turn one logical operation into a provider-side 429 burst.
fn generation_network_gate() -> &'static Semaphore {
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    GATE.get_or_init(|| Semaphore::new(1))
}

fn quota_path() -> PathBuf {
    crate::core::paths::LocalAppPaths::state_dir().join("hubcap_generation_quota.json")
}

// ============================================================================
// Shared quota file (AetherDesk + AetherDLL)
// ============================================================================

/// Creates the lock file exclusively. On Windows the handle carries
/// FILE_FLAG_DELETE_ON_CLOSE: closing it — normally or on a crash — removes
/// the file, so a lock can never outlive its owner.
#[cfg(windows)]
fn create_lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
        .open(path)
}

#[cfg(not(windows))]
fn create_lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Cross-process guard around the quota file (Desk + AetherDLL). Acquired by
/// creating the sibling `.lock` file exclusively; released by closing the
/// handle (Windows deletes it on close; other platforms remove it). A lock
/// older than `QUOTA_LOCK_STALE_AFTER` is treated as abandoned and broken —
/// a live holder's critical section lasts milliseconds, so this only fires
/// when the previous holder died.
struct QuotaFileGuard {
    lock_path: PathBuf,
    _file: std::fs::File,
}

impl QuotaFileGuard {
    fn acquire(quota_path: &Path) -> Result<Self, String> {
        let lock_path = quota_path.with_extension("lock");
        if let Some(parent) = lock_path.parent() {
            // The state directory may not exist on a fresh install yet.
            let _ = std::fs::create_dir_all(parent);
        }
        let deadline = Instant::now() + QUOTA_LOCK_TIMEOUT;
        loop {
            match create_lock_file(&lock_path) {
                Ok(file) => return Ok(Self {
                    lock_path,
                    _file: file,
                }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&lock_path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .map(|age| age > QUOTA_LOCK_STALE_AFTER)
                        .unwrap_or(false);
                    if stale {
                        // The previous holder died without releasing: break the
                        // lock and retry. A live holder keeps the file, so the
                        // remove simply re-races on the next iteration.
                        let _ = std::fs::remove_file(&lock_path);
                        crate::desk_log_warn!(
                            "hubcap",
                            "Broke stale Hubcap quota lock path={}",
                            lock_path.display()
                        );
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "Hubcap quota lock stayed busy for {}s: {}",
                            QUOTA_LOCK_TIMEOUT.as_secs(),
                            lock_path.display()
                        ));
                    }
                    std::thread::sleep(QUOTA_LOCK_RETRY_INTERVAL);
                }
                Err(error) => {
                    return Err(format!(
                        "Could not open Hubcap quota lock {}: {error}",
                        lock_path.display()
                    ));
                }
            }
        }
    }
}

impl Drop for QuotaFileGuard {
    fn drop(&mut self) {
        // Windows: the DELETE_ON_CLOSE handle removed the file when `_file`
        // closed. Other platforms remove it explicitly.
        #[cfg(not(windows))]
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Returns the fixed-EST calendar date for a Unix timestamp. This deliberately
/// does not use the host timezone or daylight-saving rules: the API contract
/// says the reset is midnight UTC-5.
fn est_day_from_unix(timestamp: i64) -> String {
    let days = (timestamp + EST_OFFSET_SECONDS).div_euclid(86_400);
    // Howard Hinnant's civil_from_days, with a Unix epoch offset.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096).div_euclid(365);
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let d = doy - (153 * mp + 2).div_euclid(5) + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}")
}

pub(crate) fn current_est_day() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    est_day_from_unix(timestamp)
}

fn quota_limit(kind: GenerationKind) -> u32 {
    match kind {
        GenerationKind::Game => MAX_GAME_GENERATIONS_PER_DAY,
        GenerationKind::Workshop => MAX_WORKSHOP_GENERATIONS_PER_DAY,
    }
}

fn quota_label(kind: GenerationKind) -> &'static str {
    match kind {
        GenerationKind::Game => "game-manifest",
        GenerationKind::Workshop => "Workshop-manifest",
    }
}

fn quota_slot(quota: &mut PersistedQuota, kind: GenerationKind) -> &mut u32 {
    match kind {
        GenerationKind::Game => &mut quota.game_used,
        GenerationKind::Workshop => &mut quota.workshop_used,
    }
}

/// Reads the persisted quota, resetting it when the fixed-EST day rolled
/// over. Any read/parse failure yields a fresh zeroed quota for today:
/// accounting is best-effort and must never wedge generation.
fn read_quota(quota_path: &Path) -> PersistedQuota {
    let today = current_est_day();
    let fresh = || PersistedQuota {
        day: today.clone(),
        ..PersistedQuota::default()
    };
    let Ok(bytes) = std::fs::read(quota_path) else {
        return fresh();
    };
    match serde_json::from_slice::<PersistedQuota>(&bytes) {
        Ok(quota) if quota.day == today => quota,
        Ok(_) => fresh(),
        Err(error) => {
            crate::desk_log_warn!(
                "hubcap",
                "Quota state parse failed path={}: {}",
                quota_path.display(),
                error
            );
            fresh()
        }
    }
}

fn write_quota(quota_path: &Path, quota: &PersistedQuota) -> Result<(), String> {
    if let Some(parent) = quota_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create Hubcap quota directory: {e}"))?;
    }
    let temporary = quota_path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(quota)
        .map_err(|e| format!("Could not serialize Hubcap quota state: {e}"))?;
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not write Hubcap quota state: {e}"))?;
    std::fs::rename(&temporary, quota_path)
        .map_err(|e| format!("Could not commit Hubcap quota state: {e}"))
}

/// Blocking reserve: one read-modify-write cycle under the cross-process
/// lock. `Err` means the budget is exhausted OR the shared state is
/// unavailable — both must block the generation.
pub(crate) fn reserve_quota_at(quota_path: &Path, kind: GenerationKind) -> Result<(), String> {
    let _guard = QuotaFileGuard::acquire(quota_path)?;
    let mut quota = read_quota(quota_path);
    let used = *quota_slot(&mut quota, kind);
    let limit = quota_limit(kind);
    crate::desk_log_debug!(
        "hubcap",
        "Quota check kind={:?} day={} used={} limit={} remaining={}",
        kind,
        quota.day,
        used,
        limit,
        limit.saturating_sub(used)
    );
    if used >= limit {
        crate::desk_log_warn!(
            "hubcap",
            "Quota exhausted kind={:?} day={} used={} limit={}",
            kind,
            quota.day,
            used,
            limit
        );
        return Err(format!(
            "Hubcap daily {} generation limit reached ({limit}; resets at midnight EST).",
            quota_label(kind)
        ));
    }
    *quota_slot(&mut quota, kind) = used + 1;
    write_quota(quota_path, &quota)?;
    crate::desk_log_info!(
        "hubcap",
        "Quota reserved kind={:?} day={} used={} remaining={}",
        kind,
        quota.day,
        used + 1,
        limit.saturating_sub(used + 1)
    );
    Ok(())
}

/// Blocking release: returns one reserved unit (best effort, never fails the
/// caller — a lost unit only under-counts).
pub(crate) fn release_quota_at(quota_path: &Path, kind: GenerationKind) {
    let guard = match QuotaFileGuard::acquire(quota_path) {
        Ok(guard) => guard,
        Err(error) => {
            crate::desk_log_error!(
                "hubcap",
                "Quota release could not acquire the lock kind={:?}: {}",
                kind,
                error
            );
            return;
        }
    };
    let mut quota = read_quota(quota_path);
    let used = *quota_slot(&mut quota, kind);
    *quota_slot(&mut quota, kind) = used.saturating_sub(1);
    if let Err(error) = write_quota(quota_path, &quota) {
        crate::desk_log_error!(
            "hubcap",
            "Quota release persistence failed kind={:?}: {}",
            kind,
            error
        );
    } else {
        crate::desk_log_info!(
            "hubcap",
            "Quota released kind={:?} day={} game_used={} workshop_used={}",
            kind,
            quota.day,
            quota.game_used,
            quota.workshop_used
        );
    }
    drop(guard);
}

/// Reserves one unit of the shared daily budget (see `reserve_quota_at`).
/// Runs on the blocking pool: the lock loop may sleep while contending with
/// AetherDLL.
async fn reserve(kind: GenerationKind) -> Result<QuotaReservation, String> {
    let quota_path = quota_path();
    let reserved = tauri::async_runtime::spawn_blocking(move || {
        reserve_quota_at(&quota_path, kind)
    })
    .await
    .map_err(|error| format!("Hubcap quota reservation task failed: {error}"))?;
    reserved.map(|_| QuotaReservation { kind })
}

/// Returns one reserved unit after a failed generation.
async fn release(reservation: QuotaReservation) {
    let quota_path = quota_path();
    let kind = reservation.kind;
    if let Err(error) =
        tauri::async_runtime::spawn_blocking(move || release_quota_at(&quota_path, kind)).await
    {
        crate::desk_log_error!(
            "hubcap",
            "Quota release task failed kind={:?}: {}",
            reservation.kind,
            error
        );
    }
}

async fn wait_for_generation_cadence() {
    let shared = state();
    let mut last = shared.last_generation_start.lock().await;
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < GENERATION_MIN_INTERVAL {
            tokio::time::sleep(GENERATION_MIN_INTERVAL - elapsed).await;
        }
    }
    *last = Some(Instant::now());
}

/// Outcome of the budget step: either generation may proceed (with an owned
/// reservation to release on failure, or no budget needed at all), or the
/// request is rejected before any network traffic.
enum Budget {
    Allowed(Option<QuotaReservation>),
    Rejected(String),
}

/// Process-wide request scheduler. `fetch` runs exactly once for a key, while
/// all other callers wait for and receive the same result. Depending on
/// `policy` the leader also holds one unit of the shared daily budget for the
/// duration of the request.
pub async fn deduplicated_generation<F, Fut>(
    key: GenerationKey,
    policy: QuotaPolicy,
    fetch: F,
) -> Result<Vec<u8>, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let shared = state();
    let (slot, leader) = {
        let mut inflight = shared.inflight.lock().await;
        if let Some(slot) = inflight.get(&key) {
            crate::desk_log_debug!(
                "hubcap",
                "Generation deduplicated waiter policy={:?} key={:?}",
                policy,
                key
            );
            (Arc::clone(slot), false)
        } else {
            let slot = Arc::new(InflightSlot::default());
            inflight.insert(key.clone(), Arc::clone(&slot));
            crate::desk_log_debug!(
                "hubcap",
                "Generation registered leader policy={:?} key={:?}",
                policy,
                key
            );
            (slot, true)
        }
    };

    if !leader {
        loop {
            let notified = slot.notify.notified();
            if let Some(completed) = slot.result.lock().await.clone() {
                crate::desk_log_debug!(
                    "hubcap",
                    "Generation deduplicated waiter completed policy={:?} key={:?} success={}",
                    policy,
                    key,
                    completed.result.is_ok()
                );
                return completed.result;
            }
            notified.await;
        }
    }

    let budget = match policy {
        QuotaPolicy::ManifestDownload => Budget::Allowed(None),
        QuotaPolicy::Generate(kind) => match reserve(kind).await {
            Ok(reservation) => Budget::Allowed(Some(reservation)),
            Err(error) => {
                crate::desk_log_warn!(
                    "hubcap",
                    "Generation reservation rejected kind={:?} key={:?}: {}",
                    kind,
                    key,
                    error
                );
                Budget::Rejected(error)
            }
        },
    };

    let result = match budget {
        Budget::Rejected(error) => Err(error),
        Budget::Allowed(reservation) => {
            crate::desk_log_debug!(
                "hubcap",
                "Generation waiting for network gate policy={:?} key={:?}",
                policy,
                key
            );
            let result = match generation_network_gate().acquire().await {
                Ok(_permit) => {
                    crate::desk_log_debug!(
                        "hubcap",
                        "Generation network gate acquired policy={:?} key={:?}",
                        policy,
                        key
                    );
                    wait_for_generation_cadence().await;
                    fetch().await
                }
                Err(error) => Err(format!("Hubcap generation scheduler unavailable: {error}")),
            };
            if result.is_err() {
                if let Some(reservation) = reservation {
                    release(reservation).await;
                }
            }
            crate::desk_log_info!(
                "hubcap",
                "Generation leader completed policy={:?} key={:?} success={}",
                policy,
                key,
                result.is_ok()
            );
            result
        }
    };

    *slot.result.lock().await = Some(CompletedRequest {
        result: result.clone(),
    });
    slot.notify.notify_waiters();
    shared.inflight.lock().await.remove(&key);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_est_midnight_is_used() {
        // 2024-01-02 04:59 UTC is still 2024-01-01 at UTC-5.
        assert_eq!(est_day_from_unix(1_704_171_540), "2024-01-01");
        // 05:00 UTC crosses the fixed EST midnight.
        assert_eq!(est_day_from_unix(1_704_172_800), "2024-01-02");
    }

    #[test]
    fn key_shapes_are_distinct() {
        assert_ne!(
            GenerationKey::Single { depot_id: 1, manifest_id: 2 },
            GenerationKey::Workshop { workshop_id: 2 }
        );
    }
}
