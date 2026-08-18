use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::core::paths::LocalAppPaths;
use crate::manifest::pins::DepotManifestPin;
use crate::versioning::cache::now_unix;

const QUEUE_FILE_NAME: &str = "pending_acf_edits.json";
const QUEUE_SCHEMA_VERSION: u32 = 1;
const MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;
const RETRY_INTERVAL_SECS: u64 = 30;

/// An ACF build edit that could not be applied right away (ACF missing until
/// the game is downloaded, or held open by a running Steam). Retried in the
/// background until it sticks or expires — same strategy as SFF's
/// `acf_pending_queue.py`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAcfEdit {
    pub app_id: u32,
    pub build_id: u64,
    pub pins: Vec<DepotManifestPin>,
    pub steam_path: String,
    pub library_path: String,
    pub queued_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct QueueFile {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    edits: Vec<PendingAcfEdit>,
}

fn queue_path() -> PathBuf {
    LocalAppPaths::data_root().join(QUEUE_FILE_NAME)
}

fn load() -> Vec<PendingAcfEdit> {
    let Ok(content) = fs::read_to_string(queue_path()) else {
        return Vec::new();
    };
    match serde_json::from_str::<QueueFile>(&content) {
        Ok(file) if file.schema_version == QUEUE_SCHEMA_VERSION => file.edits,
        _ => Vec::new(),
    }
}

fn save(edits: &[PendingAcfEdit]) -> Result<(), String> {
    let path = queue_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create data dir: {e}"))?;
    }
    let file = QueueFile {
        schema_version: QUEUE_SCHEMA_VERSION,
        edits: edits.to_vec(),
    };
    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| format!("Failed to serialize pending edits: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("Failed to write pending edits: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("Failed to commit pending edits: {e}"))
}

/// Queue (or refresh) a pending ACF edit, deduplicated by (app_id, build_id).
pub fn enqueue(edit: PendingAcfEdit) -> Result<(), String> {
    let mut edits = load();
    let queued_at = now_unix();
    for existing in edits.iter_mut() {
        if existing.app_id == edit.app_id && existing.build_id == edit.build_id {
            existing.pins = edit.pins;
            existing.steam_path = edit.steam_path;
            existing.library_path = edit.library_path;
            existing.queued_at = queued_at;
            save(&edits)?;
            crate::desk_log_info!(
                "versioning",
                "ACF edit queue: refreshed pending edit for app {} (build {})",
                edit.app_id,
                edit.build_id
            );
            return Ok(());
        }
    }
    edits.push(PendingAcfEdit { queued_at, ..edit });
    save(&edits)?;
    crate::desk_log_info!(
        "versioning",
        "ACF edit queue: queued edit for app {} (build {})",
        edit.app_id,
        edit.build_id
    );
    Ok(())
}

pub fn list() -> Vec<PendingAcfEdit> {
    load()
}

/// Background worker: every `RETRY_INTERVAL_SECS` it retries every pending
/// edit and drops the ones that stick (or expired after 7 days).
pub fn spawn_retry_worker(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(RETRY_INTERVAL_SECS)).await;
            let edits = load();
            if edits.is_empty() {
                continue;
            }
            let now = now_unix();
            let mut remaining: Vec<PendingAcfEdit> = Vec::new();
            for edit in edits {
                if now.saturating_sub(edit.queued_at) > MAX_AGE_SECS {
                    crate::desk_log_warn!(
                        "versioning",
                        "ACF edit queue: dropped expired edit for app {} (build {})",
                        edit.app_id,
                        edit.build_id
                    );
                    continue;
                }
                // ACF I/O is blocking: offload it so the async runtime stays free.
                let result = tauri::async_runtime::spawn_blocking({
                    let edit = edit.clone();
                    move || crate::versioning::apply::try_apply_pending(&edit)
                })
                .await;
                match result {
                    Ok(Ok(true)) => {
                        crate::desk_log_info!(
                            "versioning",
                            "ACF edit queue: applied pending build {} for app {}",
                            edit.build_id,
                            edit.app_id
                        );
                        let _ = app.emit(
                            "versioning://progress",
                            serde_json::json!({
                                "appId": edit.app_id,
                                "buildId": edit.build_id,
                                "step": 100,
                                "message": "ACF updated in the background"
                            }),
                        );
                    }
                    _ => remaining.push(edit),
                }
            }
            let _ = save(&remaining);
        }
    });
}
