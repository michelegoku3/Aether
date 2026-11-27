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
        .map_err(|_| "package provider scheduler is unavailable".to_string())?;
    Ok(LatestPackageGuard {
        _app: app_guard,
        _provider: provider_guard,
    })
}

/// Gate for a provider whose build runs on the server: the per-AppID lock is
/// kept for the whole job (one game, one job), while the process-wide provider
/// lane is only held until the remote job starts and is then released with
/// [`ServerJobGuard::release_provider_lane`]. Without that split a single
/// minutes-long build would block every other provider.
pub(super) struct ServerJobGuard {
    _app: OwnedMutexGuard<()>,
    provider: Option<SemaphorePermit<'static>>,
}

impl ServerJobGuard {
    /// Frees the provider lane while keeping the per-AppID serialization.
    /// Idempotent: releasing twice is a no-op.
    pub(super) fn release_provider_lane(&mut self) {
        self.provider.take();
    }
}

pub(super) async fn acquire_server_job(app_id: u32) -> Result<ServerJobGuard, String> {
    let app_guard = latest_package_lock(app_id).await.lock_owned().await;
    let provider_guard = latest_package_gate()
        .acquire()
        .await
        .map_err(|_| "package provider scheduler is unavailable".to_string())?;
    Ok(ServerJobGuard {
        _app: app_guard,
        provider: Some(provider_guard),
    })
}

#[cfg(test)]
mod tests {
    use super::{acquire_server_job, ServerJobGuard};
    use std::time::Duration;

    /// The gate is process-wide, so tests that observe it must not overlap:
    /// cargo runs test functions in parallel by default.
    static TEST_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// The lane must not stay busy while a remote job runs, otherwise a
    /// DepotBox download of a large game would block Hubcap/Ryuu/LuaTools.
    #[tokio::test]
    async fn releasing_the_lane_lets_another_game_start() {
        let _serial = TEST_SERIAL.lock().await;
        let mut first: ServerJobGuard = acquire_server_job(1).await.unwrap();
        first.release_provider_lane();
        let second = tokio::time::timeout(Duration::from_millis(500), acquire_server_job(2)).await;
        assert!(second.is_ok(), "another game must start while the job runs");
    }

    /// Before the release the lane keeps provider starts one at a time.
    #[tokio::test]
    async fn an_unreleased_job_holds_the_provider_lane() {
        let _serial = TEST_SERIAL.lock().await;
        let first: ServerJobGuard = acquire_server_job(1).await.unwrap();
        let blocked = tokio::time::timeout(Duration::from_millis(100), acquire_server_job(2)).await;
        assert!(blocked.is_err(), "the lane must serialize provider starts");
        drop(first);
        let after = tokio::time::timeout(Duration::from_millis(1000), acquire_server_job(2)).await;
        assert!(
            after.is_ok(),
            "the lane must be reusable after the guard drops"
        );
    }

    /// The per-AppID lock survives the lane release: two clicks on the same
    /// game can never run two remote jobs.
    #[tokio::test]
    async fn one_game_runs_one_job_even_after_the_lane_is_released() {
        let _serial = TEST_SERIAL.lock().await;
        let mut first = acquire_server_job(7).await.unwrap();
        first.release_provider_lane();
        let mut waiting = tokio::spawn(acquire_server_job(7));
        let early = tokio::time::timeout(Duration::from_millis(150), &mut waiting).await;
        assert!(
            early.is_err(),
            "a second job for the same AppID must wait for the first"
        );
        drop(first);
        let late = tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .expect("the queued job must run once the first guard drops");
        assert!(late.expect("task must not panic").is_ok());
    }
}
