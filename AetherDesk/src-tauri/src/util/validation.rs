/// Validate that a Steam path string is non-empty.
///
/// Shared by every Tauri command that receives `steam_path` from the frontend.
/// Keeps the error message consistent across the codebase.
pub fn validate_steam_path(steam_path: &str) -> Result<(), String> {
    if steam_path.trim().is_empty() {
        return Err("Steam installation path is required".to_string());
    }
    Ok(())
}
