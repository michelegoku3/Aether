//! Poll-loop orchestration: scan, enqueue, run one task per lane, publish.

use super::candidates::pin_refresh_candidates;
use super::lanes::{run_pin_refresh, run_pin_sync, run_repair, run_workshop};
use super::runtime::MonitorRuntime;
use super::scans::{
    changed_against, load_state, managed_update_candidates, save_state, scan_app_manifests,
    scan_managed_luas, scan_workshop_manifests,
};
use super::scheduler::PendingTask;
use super::status::{now_epoch, with_status};
use crate::core::settings::SettingsManager;
use std::collections::HashMap;
use std::time::Duration;
use tauri::AppHandle;

const START_DELAY: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_secs(20);

pub(super) async fn run(app: AppHandle) {
    tokio::time::sleep(START_DELAY).await;
    crate::desk_log_info!(
        "hubcap-updates",
        "Steam-change synchronizer started (poll every {:?}, one task per lane per poll, local-first)",
        POLL_INTERVAL
    );
    with_status(|status| {
        status.running = true;
        status.started_epoch = Some(now_epoch());
    });

    let mut checkpoint = load_state();
    let mut runtime = MonitorRuntime::default();

    loop {
        let settings = SettingsManager::new(&app).load();
        let steam_ready = !settings.steam_path.trim().is_empty();
        let key_ready = !settings.hubcap_api_key.trim().is_empty();
        let (current_acf, current_luas, current_workshop) = if steam_ready {
            (
                scan_app_manifests(&settings.steam_path),
                scan_managed_luas(&settings.steam_path),
                scan_workshop_manifests(&settings.steam_path),
            )
        } else {
            (HashMap::new(), HashMap::new(), None)
        };

        if !checkpoint.initialized {
            checkpoint.initialized = true;
            checkpoint.processed = current_acf.clone();
            checkpoint.lua_processed = current_luas.clone();
            checkpoint.workshop_processed = current_workshop.clone();
            if let Err(error) = save_state(&checkpoint) {
                crate::desk_log_warn!(
                    "hubcap-updates",
                    "Could not initialize the synchronizer checkpoint: {}",
                    error
                );
            }
        } else {
            enqueue_changed_work(
                &mut runtime,
                &mut checkpoint,
                &current_acf,
                &current_luas,
                current_workshop.as_deref(),
                &settings.steam_path,
                key_ready,
            );
        }

        run_pin_sync(
            &mut runtime,
            &app,
            &settings.steam_path,
            &current_acf,
            &mut checkpoint,
        )
        .await;
        run_pin_refresh(&mut runtime, &app, &current_acf, &mut checkpoint).await;
        run_repair(&mut runtime, &app, &current_luas, &mut checkpoint).await;
        run_workshop(
            &mut runtime,
            &app,
            &current_workshop,
            &mut checkpoint,
        )
        .await;

        runtime.publish_status(steam_ready, key_ready, checkpoint.initialized);
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn enqueue_changed_work(
    runtime: &mut MonitorRuntime,
    checkpoint: &mut super::scans::PersistedState,
    current_acf: &HashMap<u32, String>,
    current_luas: &HashMap<u32, String>,
    current_workshop: Option<&str>,
    steam_path: &str,
    key_ready: bool,
) {
    let mut checkpoint_dirty = false;
    let changed_acf = changed_against(&checkpoint.processed, current_acf);
    for app_id in managed_update_candidates(changed_acf, steam_path) {
        runtime
            .pin_sync
            .entry(app_id)
            .or_insert_with(PendingTask::due_now);
    }

    for (app_id, fingerprint) in current_acf {
        if !runtime.pin_sync.contains_key(app_id)
            && checkpoint.processed.get(app_id) != Some(fingerprint)
            && !managed_update_candidates([*app_id], steam_path).contains(app_id)
        {
            checkpoint.processed.insert(*app_id, fingerprint.clone());
            checkpoint_dirty = true;
        }
    }

    if key_ready {
        for app_id in changed_against(&checkpoint.lua_processed, current_luas) {
            runtime
                .repairs
                .entry(app_id)
                .or_insert_with(PendingTask::due_now);
        }
        for app_id in pin_refresh_candidates(
            &checkpoint.contents_checked,
            &checkpoint.contents_fingerprint,
            current_acf,
            current_luas,
            steam_path,
        ) {
            runtime
                .pin_refresh
                .entry(app_id)
                .or_insert_with(PendingTask::due_now);
        }
    }

    if key_ready && current_workshop != checkpoint.workshop_processed.as_deref() {
        if current_workshop.is_some() {
            runtime.workshop.get_or_insert_with(PendingTask::due_now);
        } else {
            checkpoint.workshop_processed = None;
            checkpoint_dirty = true;
        }
    }
    if checkpoint_dirty {
        let _ = save_state(checkpoint);
    }
}
