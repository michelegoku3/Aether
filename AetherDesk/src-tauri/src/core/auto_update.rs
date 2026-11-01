//! Background auto-update checker.
//!
//! When the user opts into `auto_update_enabled`, this module periodically
//! polls the stable AetherDesk GitHub release channel and, when a newer
//! portable build is available, downloads and stages it. The running app is
//! then notified via a Tauri event so the UI can prompt the user to restart
//! to apply the staged update.
//!
//! The check is intentionally lightweight: it only inspects the release
//! metadata (no download until the user confirms), and it runs on a long
//! interval so it never competes with the interactive startup path.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, EventTarget};

/// How often the background checker wakes up. 4 hours is long enough to avoid
/// hammering the GitHub API and short enough that an update lands within the
/// same session for most users.
const AUTO_UPDATE_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);

/// Emitted to the frontend when a newer release has been staged and is ready
/// for the user to apply.
const AUTO_UPDATE_READY_EVENT: &str = "aetherdesk://auto-update-ready";

/// Flag so the checker only runs one poll at a time.
static CHECK_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Start the periodic auto-update checker. Safe to call multiple times: only
/// the first call actually spawns the loop.
pub fn start(app: AppHandle) {
    if CHECK_IN_FLIGHT.swap(true, Ordering::Relaxed) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            check_once(&app).await;
            tokio::time::sleep(AUTO_UPDATE_INTERVAL).await;
        }
    });
}

/// Run a single check-and-stage cycle. Skipped entirely when the user has not
/// opted into auto-updates, so a fresh install does no network work.
async fn check_once(app: &AppHandle) {
    if CHECK_IN_FLIGHT.swap(true, Ordering::Relaxed) {
        return;
    }
    let result = do_check(app).await;
    CHECK_IN_FLIGHT.store(false, Ordering::Relaxed);
    let _ = result;
}

async fn do_check(app: &AppHandle) -> Result<(), String> {
    let settings = crate::core::settings::load_settings(app);
    if !settings.auto_update_enabled {
        return Ok(());
    }

    let current_version = app.package_info().version.to_string();
    let manager = crate::updater::github::GithubReleaseManager::new();

    // Always prefer the stable channel for auto-updates: testing builds are
    // opt-in and already gated by `enable_test_updates` in the manual path.
    let release = manager.fetch_latest_desk_release("auto-update").await?;
    if !crate::updater::github::GithubReleaseManager::latest_is_newer_than(
        &current_version,
        &release.tag_name,
    ) {
        return Ok(());
    }

    // Stage the update; if staging fails we log and move on rather than
    // surfacing a hard error for a background feature.
    match crate::updater::desk::prepare_update(app).await {
        Ok(Some(prepared)) => {
            crate::desk_log_info!(
                "updater",
                "Auto-update staged: {} (current: {})",
                release.tag_name,
                current_version
            );
            // Tell the UI to prompt the user to restart. The payload carries
            // the install root so the UI can show a friendly message.
            let _ = app.emit_to(
                EventTarget::app(),
                AUTO_UPDATE_READY_EVENT,
                serde_json::json!({
                    "install_root": prepared.install_root.to_string_lossy().into_owned(),
                    "tag_name": release.tag_name,
                }),
            );
        }
        Ok(None) => {
            // Up to date or test channel not newer; nothing to do.
        }
        Err(error) => {
            crate::desk_log_warn!(
                "updater",
                "Auto-update staging failed: {error}"
            );
        }
    }

    Ok(())
}