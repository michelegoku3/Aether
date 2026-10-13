//! Tauri commands for retrieving and clearing AetherDesk session logs (`desk.log`),
//! AetherDLL logs (`main.log`) and UCOnline2 logs (`uc_online2.log` in %TEMP%).

use std::path::PathBuf;

/// Lines returned when the caller does not specify a window size.
const DEFAULT_TAIL_LINES: usize = 1_000;

/// Hard ceiling for `get_recent_log_lines`: bounds the merged Vec so a single
/// IPC call cannot OOM the backend. Full-file reads stay available to the
/// export commands, which intentionally pass `usize::MAX` to the readers.
const MAX_TAIL_LINES: usize = 10_000;

fn clamp_tail_lines(requested: Option<usize>) -> usize {
    requested.unwrap_or(DEFAULT_TAIL_LINES).clamp(1, MAX_TAIL_LINES)
}

/// UCOnline2 writes its log to `%TEMP%\uc_online2.log` (single file, appended
/// while any UCO2 game runs).
fn uco2_log_path() -> PathBuf {
    std::env::temp_dir().join("uc_online2.log")
}

/// Removes the UCOnline2 log file. Called at AetherDesk startup so the log is
/// always clean and recreated automatically by UCO2 on the next game run.
pub fn clear_uco2_log_file() {
    let _ = std::fs::remove_file(uco2_log_path());
}

#[tauri::command]
pub async fn get_recent_log_lines(
    app: tauri::AppHandle,
    tail_lines: Option<usize>,
    source: Option<String>,
) -> Result<Vec<String>, String> {
    // Default window when the caller does not ask for a size. The Logs view
    // asks explicitly (LogView::TAIL_LINES), so this is the ceiling for any
    // other caller and the safety net for a missing argument. Clamped so no
    // IPC caller (including a DevTools console) can request an unbounded
    // vector and OOM the backend: merge_tagged allocates desk+dll+uco2.
    let limit = clamp_tail_lines(tail_lines);
    let mode = source.unwrap_or_else(|| "desk".to_string()).to_lowercase();
    tauri::async_runtime::spawn_blocking(move || read_log_lines(&app, limit, &mode))
        .await
        .map_err(|e| format!("Log read task failed: {e}"))?
}

#[tauri::command]
pub async fn clear_session_log(
    app: tauri::AppHandle,
    source: Option<String>,
) -> Result<String, String> {
    let mode = source.unwrap_or_else(|| "desk".to_string()).to_lowercase();
    tauri::async_runtime::spawn_blocking(move || clear_log_lines(&app, &mode))
        .await
        .map_err(|e| format!("Log clear task failed: {e}"))?
}

/// Synchronous read used inside `spawn_blocking` (never blocks the runtime).
fn read_log_lines(
    app: &tauri::AppHandle,
    limit: usize,
    mode: &str,
) -> Result<Vec<String>, String> {
    if mode == "dll" {
        return Ok(read_dll_tail_lines(app, limit));
    }
    if mode == "uco2" {
        return Ok(read_uco2_tail_lines(limit));
    }

    let desk_lines = crate::core::logger::read_tail_lines(limit)?;
    if mode == "desk" {
        return Ok(desk_lines);
    }

    // Both (All): tag desk lines with [DESK], DLL lines with [DLL ] and UCO2
    // lines with [UCO2], merge and sort chronologically.
    let dll_lines = read_dll_tail_lines(app, limit);
    let uco2_lines = read_uco2_tail_lines(limit);
    Ok(merge_tagged(desk_lines, dll_lines, uco2_lines, limit))
}

/// Merges the three sources' lines with [DESK]/[DLL ]/[UCO2] tags, ordered
/// chronologically, truncated to the last `limit` lines.
fn merge_tagged(
    desk_lines: Vec<String>,
    dll_lines: Vec<String>,
    uco2_lines: Vec<String>,
    limit: usize,
) -> Vec<String> {
    let mut merged = Vec::with_capacity(desk_lines.len() + dll_lines.len() + uco2_lines.len());
    for line in desk_lines {
        if let Some(pos) = line.find(']') {
            let (ts_part, rest) = line.split_at(pos + 1);
            merged.push(format!("{} [DESK]{}", ts_part, rest));
        } else {
            merged.push(format!("[DESK] {}", line));
        }
    }
    for line in dll_lines {
        if let Some(pos) = line.find(']') {
            let (ts_part, rest) = line.split_at(pos + 1);
            merged.push(format!("{} [DLL ]{}", ts_part, rest));
        } else {
            merged.push(format!("[DLL ] {}", line));
        }
    }
    for line in uco2_lines {
        if let Some(pos) = line.find(']') {
            let (ts_part, rest) = line.split_at(pos + 1);
            merged.push(format!("{} [UCO2]{}", ts_part, rest));
        } else {
            merged.push(format!("[UCO2] {}", line));
        }
    }

    merged.sort_by_key(|a| sort_key(a));

    if merged.len() <= limit {
        merged
    } else {
        merged[merged.len() - limit..].to_vec()
    }
}

/// Synchronous clear used inside `spawn_blocking`.
fn clear_log_lines(app: &tauri::AppHandle, mode: &str) -> Result<String, String> {
    if mode == "desk" || mode == "both" {
        crate::core::logger::clear_current_log();
    }
    if mode == "dll" || mode == "both" {
        clear_dll_log(app);
    }
    if mode == "uco2" || mode == "both" {
        clear_uco2_log_file();
    }
    Ok("Session log cleared.".to_string())
}

#[tauri::command]
pub fn set_session_log_level(
    app: tauri::AppHandle,
    level: String,
) -> Result<String, String> {
    let lower = level.trim().to_lowercase();
    if !matches!(lower.as_str(), "trace" | "debug" | "info" | "warn" | "error" | "off") {
        return Err("Unsupported log level".into());
    }
    crate::core::migration::ensure_aethercore_bridge(&app);
    for path in crate::core::presence_config::aethercore_toml_paths(&app) {
        crate::core::config_document::set_value(&path, "log", "level", toml_edit::Value::from(lower.as_str()))?;
    }
    crate::core::logger::set_level_from_str(&lower);

    crate::desk_log_info!("logs", "Set logging level to '{}' for Desk and DLL (aethercore.toml)", lower);
    Ok(format!("Logging level set to '{}' for Desk and DLL.", lower))
}

fn read_uco2_tail_lines(limit: usize) -> Vec<String> {
    let path = uco2_log_path();
    if !path.is_file() {
        return Vec::new();
    }
    // Bounded from-the-end read (see `tail_file_lines`); never fails the UI.
    // Tag normalization stays in one place (`normalize_uco2_content`).
    let lines = crate::core::logger::tail_file_lines(&path, limit).unwrap_or_default();
    if lines.is_empty() {
        return lines;
    }
    normalize_uco2_content(&lines.join("\n"))
        .lines()
        .map(str::to_string)
        .collect()
}

/// Normalizes the UCO2 log content on the AetherDesk side (without rebuilding
/// the DLL): message prefix "[UCOnline2]" -> "[UCO2]". The full timestamp
/// ("[YYYY-MM-DD HH:MM:SS.mmm]") is kept on purpose: the "both" view sorts
/// Desk and UCO2 lines together, and without the date cross-day ordering
/// would be wrong (see `sort_key`).
fn normalize_uco2_content(content: &str) -> String {
    content
        .lines()
        .map(|line| line.replace("[UCOnline2]", "[UCO2]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Chronological sort key: the first bracket content, as-is. Every timestamp
/// is fixed-width ISO (`YYYY-MM-DD HH:MM:SS.mmm`), so lexicographic order
/// matches chronological order — across days and across sources alike.
fn sort_key(line: &str) -> String {
    line.split(']')
        .next()
        .unwrap_or("")
        .trim_start_matches('[')
        .trim()
        .to_string()
}

fn read_dll_tail_lines(app: &tauri::AppHandle, limit: usize) -> Vec<String> {
    // `require_steam_path` gives us a validated, normalised path (or an empty
    // string if none is configured); in the empty case the candidate-dir
    // loop below simply finds no main.log and returns an empty vector.
    let steam_path = crate::core::settings::require_steam_path(app)
        .unwrap_or_default();
    let steam_path_buf = PathBuf::from(&steam_path);
    let desk_log_dir = crate::core::paths::LocalAppPaths::data_root().join("logs");
    let install_root = crate::core::paths::LocalAppPaths::install_root();

    for dir in [
        steam_path_buf.join("aethercore"),
        steam_path_buf.join("AetherDLL"),
        steam_path_buf.join("logs"),
        steam_path_buf.clone(),
        desk_log_dir,
        install_root,
    ] {
        let path = dir.join("main.log");
        if path.is_file() {
            // Bounded read; on failure keep looking in the next candidate dir,
            // exactly like before (a locked/huge file must not hide the others).
            if let Ok(lines) = crate::core::logger::tail_file_lines(&path, limit) {
                return lines;
            }
        }
    }
    Vec::new()
}

fn clear_dll_log(app: &tauri::AppHandle) {
    let steam_path = crate::core::settings::require_steam_path(app)
        .unwrap_or_default();
    let steam_path_buf = PathBuf::from(&steam_path);
    let desk_log_dir = crate::core::paths::LocalAppPaths::data_root().join("logs");
    let install_root = crate::core::paths::LocalAppPaths::install_root();

    for dir in [
        steam_path_buf.join("aethercore"),
        steam_path_buf.join("AetherDLL"),
        steam_path_buf.join("logs"),
        steam_path_buf.clone(),
        desk_log_dir,
        install_root,
    ] {
        let path = dir.join("main.log");
        if path.is_file() {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path);
        }
    }
}

#[cfg(target_os = "windows")]
fn current_time_filename_str() -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: SYSTEMTIME is a plain Windows FFI struct, so zero initialization is valid before GetLocalTime fills it.
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: st is valid writable storage with the SYSTEMTIME layout required by GetLocalTime.
    unsafe { GetLocalTime(&mut st) };
    format!("{:02}-{:02}-{:02}", st.wHour, st.wMinute, st.wSecond)
}

#[cfg(not(target_os = "windows"))]
fn current_time_filename_str() -> String {
    "00-00-00".to_string()
}

/// Appends the current HH-MM-SS time to a log file name, matching the time
/// format used in the exported .zip name (e.g. `desk.log.txt` becomes
/// `desk.log_14-32-05.txt`).
fn with_time_suffix(file_name: &str, time: &str) -> String {
    match file_name.rfind('.') {
        Some(idx) if idx > 0 => {
            let (stem, ext) = file_name.split_at(idx);
            format!("{stem}_{time}{ext}")
        }
        _ => format!("{file_name}_{time}"),
    }
}

#[tauri::command]
pub async fn export_logs_bundle(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || export_logs_bundle_sync(&app))
        .await
        .map_err(|e| format!("Log export task failed: {e}"))?
}

/// Synchronous export (inside `spawn_blocking`): zip creation and file copies
/// must not block the async runtime.
fn export_logs_bundle_sync(app: &tauri::AppHandle) -> Result<String, String> {
    crate::desk_log_info!("logs", "Starting export of AetherDesk, AetherDLL and UCOnline2 session logs bundle");
    let stage_dir = std::env::temp_dir().join(format!("aether_logs_export_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage_dir);
    std::fs::create_dir_all(&stage_dir)
        .map_err(|e| format!("Failed to create temporary export directory: {e}"))?;

    let time = current_time_filename_str();
    let timed = |name: &str| with_time_suffix(name, &time);

    let desk_log_dir = crate::core::paths::LocalAppPaths::data_root().join("logs");
    let install_root = crate::core::paths::LocalAppPaths::install_root();
    let steam_path = crate::core::settings::require_steam_path(app)
        .unwrap_or_default();
    let steam_path_buf = PathBuf::from(&steam_path);

    let mut copied = 0;

    // 1. AetherDesk logs
    for (src, dest_name) in [
        (desk_log_dir.join("desk.log"), "desk.log.txt"),
        (desk_log_dir.join("desk.log.last"), "desk.log.last.txt"),
        (desk_log_dir.join("status.json"), "desk_status.json.txt"),
    ] {
        if src.is_file()
            && std::fs::copy(&src, stage_dir.join(timed(dest_name))).is_ok() {
                copied += 1;
            }
    }

    // 2. AetherDLL logs across all candidate directories
    let candidate_dirs = [
        steam_path_buf.join("aethercore"),
        steam_path_buf.join("AetherDLL"),
        steam_path_buf.join("logs"),
        steam_path_buf.clone(),
        desk_log_dir,
        install_root,
    ];
    for dir in &candidate_dirs {
        for (file_name, dest_name) in [
            ("main.log", "aetherdll_main.log.txt"),
            ("main.log.last", "aetherdll_main.log.last.txt"),
            ("status.json", "aetherdll_status.json.txt"),
        ] {
            let src = dir.join(file_name);
            let dest = stage_dir.join(timed(dest_name));
            if src.is_file() && !dest.exists() && std::fs::copy(&src, &dest).is_ok() {
                copied += 1;
            }
        }
    }

    // 3. UCOnline2 log (%TEMP%\uc_online2.log) — normalized like the viewer
    //    (short timestamp + [UCO2] prefix).
    let uco2_src = uco2_log_path();
    if uco2_src.is_file() {
        if let Ok(content) = std::fs::read_to_string(&uco2_src) {
            let normalized = normalize_uco2_content(&content);
            let dest = stage_dir.join(timed("uc_online2.log.txt"));
            if std::fs::write(&dest, normalized).is_ok() {
                copied += 1;
            }
        }
    }

    let downloads_dir = dirs::download_dir().unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });
    let zip_name = format!("AetherLogs_{}.zip", time);
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

/// Downloads a single log document (the view's selected source) into the
/// Downloads folder, with the same timestamped name as the zip
/// (e.g. `desk.log_14-32-05.txt`). For "both"/"all" it creates the single
/// `all.log_<time>.txt` document with the tagged lines, same procedure as
/// the others.
#[tauri::command]
pub async fn export_log_source(
    app: tauri::AppHandle,
    source: Option<String>,
) -> Result<String, String> {
    let mode = source.unwrap_or_else(|| "desk".to_string()).to_lowercase();
    tauri::async_runtime::spawn_blocking(move || export_log_source_sync(&app, &mode))
        .await
        .map_err(|e| format!("Log export task failed: {e}"))?
}

fn export_log_source_sync(app: &tauri::AppHandle, mode: &str) -> Result<String, String> {
    let time = current_time_filename_str();
    let (base_name, content) = match mode {
        "dll" => (
            "aetherdll_main.log.txt",
            read_dll_tail_lines(app, usize::MAX).join("\n"),
        ),
        "uco2" => (
            "uc_online2.log.txt",
            read_uco2_tail_lines(usize::MAX).join("\n"),
        ),
        "both" => {
            let desk = crate::core::logger::read_tail_lines(usize::MAX)?;
            let dll = read_dll_tail_lines(app, usize::MAX);
            let uco2 = read_uco2_tail_lines(usize::MAX);
            (
                "all.log.txt",
                merge_tagged(desk, dll, uco2, usize::MAX).join("\n"),
            )
        }
        _ => ("desk.log.txt", crate::core::logger::read_tail_lines(usize::MAX)?.join("\n")),
    };

    let file_name = with_time_suffix(base_name, &time);
    let downloads_dir = dirs::download_dir().unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });
    let dest = downloads_dir.join(&file_name);
    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::write(&dest, content)
        .map_err(|e| format!("Failed to write {}: {}", dest.display(), e))?;

    crate::desk_log_info!("logs", "Exported log source '{}' to {}", mode, dest.display());
    Ok(format!("Exported {} to {}", file_name, dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_limit_is_bounded_for_ipc_callers() {
        assert_eq!(clamp_tail_lines(None), DEFAULT_TAIL_LINES);
        assert_eq!(clamp_tail_lines(Some(500)), 500);
        assert_eq!(clamp_tail_lines(Some(0)), 1);
        assert_eq!(clamp_tail_lines(Some(usize::MAX)), MAX_TAIL_LINES);
        assert_eq!(clamp_tail_lines(Some(1_000_000)), MAX_TAIL_LINES);
    }

    #[test]
    fn uco2_normalize_keeps_the_date_and_shortens_the_tag() {
        let out = normalize_uco2_content("[2026-08-17 12:45:40.924] [UCOnline2] hello");
        assert_eq!(out, "[2026-08-17 12:45:40.924] [UCO2] hello");
    }

    #[test]
    fn merged_logs_sort_across_days_and_sources() {
        let desk = vec![
            "[2026-08-18 00:00:01.000] [PID:00001] [TID:00001] [INFO ] [Desk] second day".to_string(),
        ];
        let dll = vec![];
        let uco2 = vec![
            "[2026-08-17 23:59:59.999] [UCO2] first day".to_string(),
        ];
        let merged = merge_tagged(desk, dll, uco2, 100);
        assert_eq!(merged.len(), 2);
        assert!(merged[0].contains("first day"), "unexpected order: {merged:?}");
        assert!(merged[1].contains("second day"), "unexpected order: {merged:?}");
    }
}
