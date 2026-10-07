//! Concurrency policy for latest-package provider requests.
//!
//! Duplicate requests for one AppID are coalesced by a per-game lock. A
//! process-wide provider lane prevents package ZIP requests for different
//! games from becoming a rate-limit burst.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, Semaphore, SemaphorePermit};

type LatestPackageLocks = AsyncMutex<HashMap<u32, Arc<AsyncMutex<()>>>>;

fn latest_package_gate() -> &'static Semaphore {
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    GATE.get_or_init(|| Semaphore::new(1))
}

fn latest_package_locks() -> &'static LatestPackageLocks {
    static LOCKS: OnceLock<LatestPackageLocks> = OnceLock::new();
    LOCKS.get_or_init(|| AsyncMutex::new(HashMap::new()))
}

async fn latest_package_lock(app_id: u32) -> Arc<AsyncMutex<()>> {
    let mut locks = latest_package_locks().lock().await;
    locks
        .entry(app_id)
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

/// Holds both guards in declaration order for the duration of one provider
/// acquisition. The per-AppID guard is owned so its backing `Arc` cannot be
/// dropped while the request is in progress.
pub(super) struct LatestPackageGuard {
    _app: OwnedMutexGuard<()>,
    _provider: SemaphorePermit<'static>,
}

pub(super) async fn acquire_latest_package(app_id: u32) -> Result<LatestPackageGuard, String> {
    let app_guard = latest_package_lock(app_id).await.lock_owned().await;
    let provider_guard = latest_package_gate()
        .acquire()
        .await
        .map_err(|_| "Hubcap package scheduler is unavailable".to_string())?;
    Ok(LatestPackageGuard {
        _app: app_guard,
        _provider: provider_guard,
    })
}
