//! Authenticated Hubcap generation scheduling.
//!
//! This module owns the process-wide quota and deduplication policy. It never
//! logs API keys, never requests a manifest that already exists locally (the
//! caller performs the exact local check), and coalesces identical concurrent
//! generation requests so one user action cannot spend the same quota twice.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenerationKind {
    Game,
    Workshop,
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
    quota: Mutex<PersistedQuota>,
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

fn current_est_day() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    est_day_from_unix(timestamp)
}

fn load_persisted() -> PersistedQuota {
    let path = quota_path();
    let Ok(bytes) = std::fs::read(&path) else {
        crate::desk_log_debug!("hubcap", "Quota state cache miss path={}", path.display());
        return PersistedQuota {
            day: current_est_day(),
            ..PersistedQuota::default()
        };
    };
    let mut state: PersistedQuota = match serde_json::from_slice(&bytes) {
        Ok(state) => state,
        Err(error) => {
            crate::desk_log_warn!("hubcap", "Quota state cache parse failed path={}: {}", path.display(), error);
            PersistedQuota::default()
        }
    };
    let day = current_est_day();
    if state.day != day {
        state = PersistedQuota {
            day,
            ..PersistedQuota::default()
        };
    }
    state
}

fn persist_quota(quota: &PersistedQuota) -> Result<(), String> {
    let path = quota_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create Hubcap quota directory: {e}"))?;
    }
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(quota)
        .map_err(|e| format!("Could not serialize Hubcap quota state: {e}"))?;
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not write Hubcap quota state: {e}"))?;
    std::fs::rename(&temporary, &path)
        .map_err(|e| format!("Could not commit Hubcap quota state: {e}"))
}

async fn reserve(kind: GenerationKind) -> Result<QuotaReservation, String> {
    let shared = state();
    let mut quota = shared.quota.lock().await;
    if quota.day.is_empty() {
        *quota = load_persisted();
    }
    let day = current_est_day();
    if quota.day != day {
        *quota = PersistedQuota {
            day,
            ..PersistedQuota::default()
        };
    }

    let (used, limit) = match kind {
        GenerationKind::Game => (quota.game_used, MAX_GAME_GENERATIONS_PER_DAY),
        GenerationKind::Workshop => (quota.workshop_used, MAX_WORKSHOP_GENERATIONS_PER_DAY),
    };
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
        let label = match kind {
            GenerationKind::Game => "game-manifest",
            GenerationKind::Workshop => "Workshop-manifest",
        };
        crate::desk_log_warn!(
            "hubcap",
            "Quota exhausted kind={:?} day={} used={} limit={}",
            kind,
            quota.day,
            used,
            limit
        );
        return Err(format!("Hubcap daily {label} generation limit reached ({limit}; resets at midnight EST)."));
    }

    match kind {
        GenerationKind::Game => quota.game_used += 1,
        GenerationKind::Workshop => quota.workshop_used += 1,
    }
    if let Err(error) = persist_quota(&quota) {
        match kind {
            GenerationKind::Game => quota.game_used = quota.game_used.saturating_sub(1),
            GenerationKind::Workshop => quota.workshop_used = quota.workshop_used.saturating_sub(1),
        }
        crate::desk_log_error!("hubcap", "Quota reservation persistence failed kind={:?}: {}", kind, error);
        return Err(error);
    }
    crate::desk_log_info!(
        "hubcap",
        "Quota reserved kind={:?} day={} used={} remaining={}",
        kind,
        quota.day,
        used + 1,
        limit.saturating_sub(used + 1)
    );
    Ok(QuotaReservation { kind })
}

async fn release(reservation: QuotaReservation) {
    let shared = state();
    let mut quota = shared.quota.lock().await;
    match reservation.kind {
        GenerationKind::Game => quota.game_used = quota.game_used.saturating_sub(1),
        GenerationKind::Workshop => quota.workshop_used = quota.workshop_used.saturating_sub(1),
    }
    if let Err(error) = persist_quota(&quota) {
        crate::desk_log_error!(
            "hubcap",
            "Quota release persistence failed kind={:?}: {}",
            reservation.kind,
            error
        );
    } else {
        crate::desk_log_info!(
            "hubcap",
            "Quota released kind={:?} day={} game_used={} workshop_used={}",
            reservation.kind,
            quota.day,
            quota.game_used,
            quota.workshop_used
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

/// Process-wide request scheduler. `fetch` runs exactly once for a key, while
/// all other callers wait for and receive the same result.
pub async fn deduplicated_generation<F, Fut>(
    key: GenerationKey,
    kind: GenerationKind,
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
                "Generation deduplicated waiter kind={:?} key={:?}",
                kind,
                key
            );
            (Arc::clone(slot), false)
        } else {
            let slot = Arc::new(InflightSlot::default());
            inflight.insert(key.clone(), Arc::clone(&slot));
            crate::desk_log_debug!(
                "hubcap",
                "Generation registered leader kind={:?} key={:?}",
                kind,
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
                    "Generation deduplicated waiter completed kind={:?} key={:?} success={}",
                    kind,
                    key,
                    completed.result.is_ok()
                );
                return completed.result;
            }
            notified.await;
        }
    }

    let result = match reserve(kind).await {
        Ok(reservation) => {
            crate::desk_log_debug!("hubcap", "Generation waiting for network gate kind={:?} key={:?}", kind, key);
            let result = match generation_network_gate().acquire().await {
                Ok(_permit) => {
                    crate::desk_log_debug!("hubcap", "Generation network gate acquired kind={:?} key={:?}", kind, key);
                    wait_for_generation_cadence().await;
                    fetch().await
                }
                Err(error) => Err(format!("Hubcap generation scheduler unavailable: {error}")),
            };
            if result.is_err() {
                release(reservation).await;
            }
            crate::desk_log_info!(
                "hubcap",
                "Generation leader completed kind={:?} key={:?} success={}",
                kind,
                key,
                result.is_ok()
            );
            result
        }
        Err(error) => {
            crate::desk_log_warn!(
                "hubcap",
                "Generation reservation rejected kind={:?} key={:?}: {}",
                kind,
                key,
                error
            );
            Err(error)
        }
    };

    *slot.result.lock().await = Some(CompletedRequest {
        result: result.clone(),
    });
    slot.notify.notify_waiters();
    shared.inflight.lock().await.remove(&key);
    result
}

/// Test-only reset hook. Production code never needs to clear quota state.
#[cfg(test)]
pub async fn reset_for_tests() {
    let shared = state();
    *shared.quota.lock().await = PersistedQuota::default();
    shared.inflight.lock().await.clear();
    *shared.last_generation_start.lock().await = None;
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
