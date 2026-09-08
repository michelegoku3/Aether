//! Window-lifecycle commands.

use tauri::Manager;

/// Force-destroys the main window, bypassing the close-requested guard.
///
/// The frontend prevents the default close only when Settings has unsaved
/// edits; once the user confirms Save / Don't Save in the modal, it calls
/// this to actually quit. Implemented as a custom command (rather than the
/// core window API from JS) so it works with zero capability grants.
#[tauri::command]
pub fn force_close_window(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?
        .destroy()
        .map_err(|e| e.to_string())
}
