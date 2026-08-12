//! Tauri commands for retrieving and clearing AetherDesk session logs (`desk.log`).
//!
//! # Purpose
//! Exposes `crate::core::logger` session logs to the frontend "Logs View" UI
//! (`activeTab === 'log'`), allowing real-time inspection and clearing.

use std::path::PathBuf;

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

#[tauri::command]
pub fn export_logs_bundle(app: tauri::AppHandle) -> Result<String, String> {
    crate::desk_log_info!("logs", "Starting export of AetherDesk and AetherDLL session logs bundle");
    let stage_dir = std::env::temp_dir().join(format!("aether_logs_export_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage_dir);
    std::fs::create_dir_all(&stage_dir)
        .map_err(|e| format!("Failed to create temporary export directory: {e}"))?;

    let desk_log_dir = crate::core::paths::LocalAppPaths::data_root().join("logs");
    let steam_path = crate::core::settings::SettingsManager::new(&app).load().steam_path;
    let dll_log_dir = PathBuf::from(steam_path).join("AetherDLL");

    let mut copied = 0;
    for (src, dest_name) in [
        (desk_log_dir.join("desk.log"), "desk.log.txt"),
        (desk_log_dir.join("desk.log.last"), "desk.log.last.txt"),
        (desk_log_dir.join("status.json"), "desk_status.json.txt"),
        (dll_log_dir.join("main.log"), "aetherdll_main.log.txt"),
        (dll_log_dir.join("main.log.last"), "aetherdll_main.log.last.txt"),
        (dll_log_dir.join("status.json"), "status.json.txt"),
    ] {
        if src.is_file() {
            if let Ok(_) = std::fs::copy(&src, stage_dir.join(dest_name)) {
                copied += 1;
            }
        }
    }

    let downloads_dir = dirs::download_dir().unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });
    let zip_path = downloads_dir.join("Aether_Logs_Bundle.zip");
    if zip_path.exists() {
        let _ = std::fs::remove_file(&zip_path);
    }

    let file = std::fs::File::create(&zip_path)
        .map_err(|e| format!("Failed to create ZIP bundle {}: {}", zip_path.display(), e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    if let Ok(entries) = std::fs::read_dir(&stage_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let _ = zip.start_file(name, options);
                    if let Ok(content) = std::fs::read(&path) {
                        let _ = std::io::Write::write_all(&mut zip, &content);
                    }
                }
            }
        }
    }
    let _ = zip.finish();
    let _ = std::fs::remove_dir_all(&stage_dir);

    crate::desk_log_info!("logs", "Successfully exported {} log file(s) into {}", copied, zip_path.display());
    Ok(format!("Exported {} log file(s) to {}", copied, zip_path.display()))
}
