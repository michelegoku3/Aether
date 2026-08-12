//! Tauri commands for retrieving and clearing AetherDesk session logs (`desk.log`).
//!
//! # Purpose
//! Exposes `crate::core::logger` session logs to the frontend "Logs View" UI
//! (`activeTab === 'log'`), allowing real-time inspection and clearing.

#[tauri::command]
pub fn get_recent_log_lines(tail_lines: Option<usize>) -> Result<Vec<String>, String> {
    let limit = tail_lines.unwrap_or(200);
    crate::core::logger::read_tail_lines(limit)
}

#[tauri::command]
pub fn clear_session_log() -> Result<String, String> {
    crate::core::logger::clear_current_log()?;
    Ok("Session log cleared.".to_string())
}
