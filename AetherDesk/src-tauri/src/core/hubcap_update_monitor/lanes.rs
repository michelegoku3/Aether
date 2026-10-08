//! Execution of one ready task per monitor lane.

use super::installed_state::sync_pins_after_steam_update;
use super::runtime::MonitorRuntime;
use super::scans::{save_state, PersistedState};
use super::scheduler::{
    clear_unresolved, mark_unresolved, reschedule, retry_delay, take_ready, PendingTask,
};
use super::status::{now_epoch, with_status};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tauri::AppHandle;

const PIN_REFRESH_MIN_GAP: Duration = Duration::from_secs(60);
const WORKSHOP_DEFERRED_RETRY_DELAY: Duration = Duration::from_secs(90);

pub(super) async fn run_pin_sync(
    runtime: &mut MonitorRuntime,
    app: &AppHandle,
    steam_path: &str,
    current_acf: &HashMap<u32, String>,
    checkpoint: &mut PersistedState,
) {
    let Some((app_id, task)) = take_ready(&mut runtime.pin_sync) else {
        return;
    };
    match sync_pins_after_steam_update(app, steam_path, app_id).await {
        Ok(realigned) => {
            if let Some(fingerprint) = current_acf.get(&app_id) {
                checkpoint.processed.insert(app_id, fingerprint.clone());
                let _ = save_state(checkpoint);
            }
            with_status(|status| {
                status.pin_sync.processed_count += 1;
                status.pin_sync.last_run_epoch = Some(now_epoch());
                status.pin_sync.last_error = None;
            });
            clear_unresolved(|status| &mut status.pin_sync, app_id);
            crate::desk_log_info!(
                "hubcap-updates",
                "Pin sync complete app_id={} realigned={}",
                app_id,
                realigned
            );
        }
        Err(error) => {
            with_status(|status| status.pin_sync.last_error = Some(error.clone()));
            crate::desk_log_warn!(
                "hubcap-updates",
                "Pin sync failed app_id={}: {}",
                app_id,
                error
            );
            let attempts = task.attempts.saturating_add(1);
            if !reschedule(&mut runtime.pin_sync, app_id, &task) {
                mark_unresolved(|status| &mut status.pin_sync, app_id, attempts);
            }
        }
    }
}

pub(super) async fn run_pin_refresh(
    runtime: &mut MonitorRuntime,
    app: &AppHandle,
    current_acf: &HashMap<u32, String>,
    checkpoint: &mut PersistedState,
) {
    let gate_open = runtime
        .last_pin_refresh_at
        .map(|last| last.elapsed() >= PIN_REFRESH_MIN_GAP)
        .unwrap_or(true);
    if !gate_open {
        return;
    }
    let Some((app_id, task)) = take_ready(&mut runtime.pin_refresh) else {
        return;
    };
    runtime.last_pin_refresh_at = Some(Instant::now());

    match crate::manifest::pin_refresh::refresh_game_pins_from_hubcap(
        app.clone(),
        app_id,
    )
    .await
    {
        Ok(report) => {
            checkpoint.contents_checked.insert(app_id, now_epoch());
            if let Some(fingerprint) = current_acf.get(&app_id) {
                checkpoint
                    .contents_fingerprint
                    .insert(app_id, fingerprint.clone());
            }
            let _ = save_state(checkpoint);
            with_status(|status| {
                status.pin_refresh.processed_count += 1;
                status.pin_refresh.last_run_epoch = Some(now_epoch());
                status.pin_refresh.last_error = None;
            });
            clear_unresolved(|status| &mut status.pin_refresh, app_id);
            crate::desk_log_info!(
                "hubcap-updates",
                "Pin refresh complete app_id={} skipped={} checked_depots={} staged={} realigned={} invalid_pin_line={:?}",
                app_id,
                report.skipped,
                report.checked_depots,
                report.staged,
                report.realigned,
                report.invalid_pin_line
            );
        }
        Err(error) => {
            with_status(|status| status.pin_refresh.last_error = Some(error.clone()));
            crate::desk_log_warn!(
                "hubcap-updates",
                "Pin refresh deferred app_id={}: {}",
                app_id,
                error
            );
            let attempts = task.attempts.saturating_add(1);
            if !reschedule(&mut runtime.pin_refresh, app_id, &task) {
                checkpoint.contents_checked.insert(app_id, now_epoch());
                if let Some(fingerprint) = current_acf.get(&app_id) {
                    checkpoint
                        .contents_fingerprint
                        .insert(app_id, fingerprint.clone());
                }
                let _ = save_state(checkpoint);
                mark_unresolved(|status| &mut status.pin_refresh, app_id, attempts);
            }
        }
    }
}

pub(super) async fn run_repair(
    runtime: &mut MonitorRuntime,
    app: &AppHandle,
    current_luas: &HashMap<u32, String>,
    checkpoint: &mut PersistedState,
) {
    let Some((app_id, task)) = take_ready(&mut runtime.repairs) else {
        return;
    };
    match crate::commands::manifests::sync_hubcap_game_manifest(app.clone(), app_id).await {
        Ok(_) => {
            if let Some(fingerprint) = current_luas.get(&app_id) {
                checkpoint.lua_processed.insert(app_id, fingerprint.clone());
                let _ = save_state(checkpoint);
            }
            with_status(|status| {
                status.repair.processed_count += 1;
                status.repair.last_run_epoch = Some(now_epoch());
                status.repair.last_error = None;
            });
            clear_unresolved(|status| &mut status.repair, app_id);
            crate::desk_log_info!(
                "hubcap-updates",
                "Local Lua manifest repair complete app_id={}",
                app_id
            );
        }
        Err(error) => {
            with_status(|status| status.repair.last_error = Some(error.clone()));
            crate::desk_log_warn!(
                "hubcap-updates",
                "Local Lua manifest repair deferred app_id={}: {}",
                app_id,
                error
            );
            let attempts = task.attempts.saturating_add(1);
            if !reschedule(&mut runtime.repairs, app_id, &task) {
                mark_unresolved(|status| &mut status.repair, app_id, attempts);
            }
        }
    }
}

pub(super) async fn run_workshop(
    runtime: &mut MonitorRuntime,
    app: &AppHandle,
    current_workshop: &Option<String>,
    checkpoint: &mut PersistedState,
) {
    let ready = runtime
        .workshop
        .as_ref()
        .map(|task| task.next_attempt <= Instant::now())
        .unwrap_or(false);
    if !ready {
        return;
    }
    let task = runtime
        .workshop
        .take()
        .expect("readiness checked above");
    match crate::commands::workshop::sync_hubcap_workshop_manifests(app.clone()).await {
        Ok(report) if report.failed == 0 && report.deferred == 0 => {
            checkpoint.workshop_processed = current_workshop.clone();
            let _ = save_state(checkpoint);
            with_status(|status| {
                status.workshop.processed_count += 1;
                status.workshop.last_run_epoch = Some(now_epoch());
                status.workshop.last_error = None;
            });
            crate::desk_log_info!(
                "hubcap-updates",
                "Workshop sync complete discovered={} generated={} cached={} local={} content_missing={}",
                report.discovered,
                report.generated,
                report.restored_from_cache,
                report.already_local,
                report.content_missing
            );
        }
        Ok(report) if report.failed == 0 => {
            crate::desk_log_info!(
                "hubcap-updates",
                "Workshop batch complete generated={} deferred={}; next batch scheduled",
                report.generated,
                report.deferred
            );
            runtime.workshop = Some(PendingTask {
                attempts: task.attempts,
                next_attempt: Instant::now() + WORKSHOP_DEFERRED_RETRY_DELAY,
            });
        }
        Ok(report) => {
            let error = format!("{} Workshop item(s) failed", report.failed);
            with_status(|status| status.workshop.last_error = Some(error.clone()));
            crate::desk_log_warn!("hubcap-updates", "Workshop sync incomplete: {}", error);
            let attempts = task.attempts.saturating_add(1);
            runtime.workshop = Some(PendingTask {
                attempts,
                next_attempt: Instant::now() + retry_delay(attempts),
            });
        }
        Err(error) => {
            with_status(|status| status.workshop.last_error = Some(error.clone()));
            crate::desk_log_warn!("hubcap-updates", "Workshop sync deferred: {}", error);
            let attempts = task.attempts.saturating_add(1);
            runtime.workshop = Some(PendingTask {
                attempts,
                next_attempt: Instant::now() + retry_delay(attempts),
            });
        }
    }
}
