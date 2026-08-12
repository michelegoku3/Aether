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

#[cfg(target_os = "windows")]
fn current_time_filename_str() -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    format!("{:02}-{:02}-{:02}", st.wHour, st.wMinute, st.wSecond)
}

#[cfg(not(target_os = "windows"))]
fn current_time_filename_str() -> String {
    "00-00-00".to_string()
}

#[tauri::command]
pub fn export_logs_bundle(app: tauri::AppHandle) -> Result<String, String> {
    crate::desk_log_info!("logs", "Starting export of AetherDesk and AetherDLL session logs bundle");
    let stage_dir = std::env::temp_dir().join(format!("aether_logs_export_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage_dir);
    std::fs::create_dir_all(&stage_dir)
        .map_err(|e| format!("Failed to create temporary export directory: {e}"))?;

    let desk_log_dir = crate::core::paths::LocalAppPaths::data_root().join("logs");
    let install_root = crate::core::paths::LocalAppPaths::install_root();
    let steam_path = crate::core::settings::SettingsManager::new(&app).load().steam_path;
    let steam_path_buf = PathBuf::from(&steam_path);

    let mut copied = 0;

    // 1. AetherDesk logs
    for (src, dest_name) in [
        (desk_log_dir.join("desk.log"), "desk.log.txt"),
        (desk_log_dir.join("desk.log.last"), "desk.log.last.txt"),
        (desk_log_dir.join("status.json"), "desk_status.json.txt"),
    ] {
        if src.is_file() {
            if let Ok(_) = std::fs::copy(&src, stage_dir.join(dest_name)) {
                copied += 1;
            }
        }
    }

    // 2. AetherDLL logs across all candidate directories
    let candidate_dirs = [
        steam_path_buf.join("aethercore"),
        steam_path_buf.join("AetherDLL"),
        steam_path_buf.join("logs"),
        steam_path_buf.clone(),
        desk_log_dir.clone(),
        install_root.clone(),
    ];
    for dir in &candidate_dirs {
        for (file_name, dest_name) in [
            ("main.log", "aetherdll_main.log.txt"),
            ("main.log.last", "aetherdll_main.log.last.txt"),
            ("status.json", "aetherdll_status.json.txt"),
        ] {
            let src = dir.join(file_name);
            let dest = stage_dir.join(dest_name);
            if src.is_file() && !dest.exists() {
                if let Ok(_) = std::fs::copy(&src, &dest) {
                    copied += 1;
                }
            }
        }
    }

    let downloads_dir = dirs::download_dir().unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });
    let zip_name = format!("AetherLogs_{}.zip", current_time_filename_str());
    let zip_path = downloads_dir.join(&zip_name);
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
