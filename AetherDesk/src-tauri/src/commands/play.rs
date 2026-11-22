//! "Play" from the library: free RAM first, then hand the game to Steam.
//!
//! Split in two commands on purpose: `preview_ram_optimizer` lets the UI show
//! exactly which processes would be closed (and which whitelist entries are
//! protected) *before* anything is signalled, and `optimize_and_launch_game`
//! performs the confirmed plan and only then launches the game.

use crate::core::ram_optimizer::{self, OptimizeOptions, OptimizeReport, ProcessEntry};
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

fn options(app: &AppHandle, min_memory_mb: Option<u32>) -> OptimizeOptions {
    let protect_dirs = configured_steam_path(app)
        .map(|path| ram_optimizer::protected_dirs(Some(&PathBuf::from(path))))
        .unwrap_or_default();
    OptimizeOptions {
        min_memory_mb: min_memory_mb.unwrap_or(150),
        protect_dirs,
    }
}

/// Dry run: the process list with its close/keep verdicts.
#[tauri::command]
pub fn preview_ram_optimizer(
    app: AppHandle,
    min_memory_mb: Option<u32>,
) -> Result<Vec<ProcessEntry>, String> {
    let opts = options(&app, min_memory_mb);
    Ok(ram_optimizer::plan(&opts))
}

/// Close the planned processes, then launch `app_id` through Steam.
#[tauri::command]
pub fn optimize_and_launch_game(
    app: AppHandle,
    app_id: u32,
    min_memory_mb: Option<u32>,
) -> Result<PlayReport, String> {
    let opts = options(&app, min_memory_mb);
    let report = ram_optimizer::optimize(&opts);
    ram_optimizer::launch_steam_game(app_id)?;
    Ok(PlayReport {
        report,
        launched: true,
        message: format!("Launched Steam app {app_id}."),
    })
}
