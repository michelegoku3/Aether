use std::fs;
use std::path::PathBuf;
use crate::core::paths::LocalAppPaths;

/// Single source of truth for the custom CSS location.
/// Stored next to `settings.json` so the user finds it easily
/// and it benefits from the same `AetherData` Defender exclusion.
pub fn custom_css_path() -> PathBuf {
    LocalAppPaths::config_dir().join("custom.css")
}

/// Default template written when the file does not exist.
/// Commented-out so toggling ON with an untouched file does nothing,
/// but the user sees an example and where to write.
const DEFAULT_TEMPLATE: &str = r#"/* AetherDesk Custom CSS
   This file is loaded only when "Enable Custom CSS" is ON in Settings.
   Write your overrides after this comment. They are injected as a
   <style id="aether-custom-css"> tag after the default theme, so they
   win by cascade order.

   Examples:
   :root { --bg-app: #0a0a0f; --color-cyan: #ff00ff; }
   .sidebar { border-right: 2px solid var(--color-cyan); }
   .store-game-card { border-radius: 16px; }
*/

"#;

/// Ensure the parent directory and the file exist.
/// If the file is missing, creates it with `DEFAULT_TEMPLATE` and returns its path.
/// If it already exists, returns the path without touching it.
/// Fail-open: directory creation errors are propagated as `Err`, but callers
/// (Tauri commands) map them to user-visible messages without crashing the app.
pub fn ensure_custom_css() -> Result<PathBuf, String> {
    let path = custom_css_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create custom.css directory: {}", e))?;
    }
    if !path.exists() {
        fs::write(&path, DEFAULT_TEMPLATE)
            .map_err(|e| format!("Failed to create custom.css: {}", e))?;
    }
    Ok(path)
}

/// Read the custom CSS file.
/// - If the file does not exist → `Ok("")` (not an error, toggle can still be ON
///   but nothing is injected — the frontend shows "empty" and offers to create).
/// - If the file is unreadable → `Err` with a human message (shown in console, not as panic).
pub fn read_custom_css() -> Result<String, String> {
    let path = custom_css_path();
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(&path).map_err(|e| format!("Failed to read custom.css: {}", e))
}
