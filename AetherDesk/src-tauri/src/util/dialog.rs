//! Shared helpers for native dialog results.
//!
//! Every picker command (`pick_*`) converts Tauri [`FilePath`] values to plain
//! strings the same way; keeping the conversion here avoids a private copy in
//! each command module (DRY).

use tauri_plugin_dialog::FilePath;

/// Convert a picked [`FilePath`] into a displayable path string.
/// Returns `None` when the path cannot be resolved (treated as "cancelled").
pub fn file_path_to_string(file_path: FilePath) -> Option<String> {
    file_path
        .into_path()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
}
