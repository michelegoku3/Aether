//! "Play" from the library: free RAM first, then hand the game to Steam.
//!
//! Split in two commands on purpose: `preview_ram_optimizer` lets the UI show
//! exactly which processes would be closed (and which whitelist entries are
//! protected) *before* anything is signalled, and `optimize_and_launch_game`
//! performs the confirmed plan and only then launches the game.

use crate::core::ram_optimizer::{self, OptimizeOptions, OptimizeReport, RamPreview};
use serde::Serialize;
use std::path::PathBuf;
use tauri::AppHandle;

use super::configured_steam_path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayReport {
    pub report: OptimizeReport,
    pub launched: bool,
    pub message: String,
}

fn options(app: &AppHandle, min_memory_mb: Option<u32>, exclude_pids: Vec<u32>) -> OptimizeOptions {
    let protect_dirs =
        ram_optimizer::protected_dirs(configured_steam_path(app).map(PathBuf::from).as_deref());
    OptimizeOptions {
        min_memory_mb: min_memory_mb.unwrap_or(150),
        protect_dirs,
        exclude_pids,
    }
}

/// Dry run: the process list with its close/keep verdicts and system RAM info.
#[tauri::command]
pub async fn preview_ram_optimizer(
    app: AppHandle,
    min_memory_mb: Option<u32>,
) -> Result<RamPreview, String> {
    let opts = options(&app, min_memory_mb, Vec::new());
    tokio::task::spawn_blocking(move || ram_optimizer::preview(&opts))
        .await
        .map_err(|error| format!("RAM preview task failed: {error}"))
}

/// Optionally close the planned processes (respecting `exclude_pids`), then launch `app_id` through Steam.
#[tauri::command]
pub async fn optimize_and_launch_game(
    app: AppHandle,
    app_id: u32,
    min_memory_mb: Option<u32>,
    skip_optimize: Option<bool>,
    exclude_pids: Option<Vec<u32>>,
) -> Result<PlayReport, String> {
    let steam_path = configured_steam_path(&app).map(PathBuf::from);
    if skip_optimize.unwrap_or(false) {
        return tokio::task::spawn_blocking(move || {
            ram_optimizer::launch_steam_game(app_id, steam_path.as_deref())?;
            Ok(PlayReport {
                report: OptimizeReport::default(),
                launched: true,
                message: format!("Launched Steam app {app_id} directly."),
            })
        })
        .await
        .map_err(|error| format!("Launch task failed: {error}"))?;
    }
    let opts = options(&app, min_memory_mb, exclude_pids.unwrap_or_default());
    tokio::task::spawn_blocking(move || {
        let report = ram_optimizer::optimize(&opts);
        ram_optimizer::launch_steam_game(app_id, steam_path.as_deref())?;
        Ok(PlayReport {
            report,
            launched: true,
            message: format!("Launched Steam app {app_id}."),
        })
    })
    .await
    .map_err(|error| format!("Optimize and launch task failed: {error}"))?
}
