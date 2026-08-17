//! Tauri commands for retrieving and clearing AetherDesk session logs (`desk.log`),
//! AetherDLL logs (`main.log`) and UCOnline2 logs (`uc_online2.log` in %TEMP%).

use std::path::PathBuf;

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
    let limit = tail_lines.unwrap_or(200);
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

/// Lettura sincrona usata dentro `spawn_blocking` (non blocca il runtime).
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

/// Unisce le righe delle tre sorgenti con tag [DESK]/[DLL ]/[UCO2],
/// ordinate cronologicamente, troncate alle ultime `limit` righe.
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

    merged.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));

    if merged.len() <= limit {
        merged
    } else {
        merged[merged.len() - limit..].to_vec()
    }
}

/// Cancellazione sincrona usata dentro `spawn_blocking`.
fn clear_log_lines(app: &tauri::AppHandle, mode: &str) -> Result<String, String> {
    if mode == "desk" || mode == "both" {
        crate::core::logger::clear_current_log()?;
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
    crate::core::logger::set_level_from_str(&lower);

    // 1. Ensure the bridge pointer desk_path.cfg and toml exist
    crate::core::migration::ensure_aethercore_bridge(&app);

    // 2. Update <install_root>\AetherData\config\aethercore.toml (new primary home)
    let config_dir = crate::core::paths::LocalAppPaths::config_dir();
    let toml_path = config_dir.join("aethercore.toml");

    let new_content = if toml_path.exists() {
        let content = std::fs::read_to_string(&toml_path).unwrap_or_default();
        if content.contains("[log]") {
            let re = regex::Regex::new(r#"(?m)^level\s*=\s*".*""#).unwrap();
            if re.is_match(&content) {
                re.replace(&content, format!("level = \"{}\"", lower)).to_string()
            } else {
                content.replace("[log]", &format!("[log]\nlevel = \"{}\"", lower))
            }
        } else {
            format!("{}\n\n[log]\nlevel = \"{}\"\nkeep_last_session = true\n", content, lower)
        }
    } else {
        format!("# AetherCore configuration.\n# Located at AetherData/config/aethercore.toml (managed by AetherDesk).\n[log]\nlevel = \"{}\"\nkeep_last_session = true\n", lower)
    };
    let _ = std::fs::write(&toml_path, new_content);

    // 3. Also update <Steam>\aethercore\aethercore.toml if present for legacy compatibility
    let steam_path = crate::core::settings::SettingsManager::new(&app).load().steam_path;
    if !steam_path.trim().is_empty() {
        let steam_toml = PathBuf::from(&steam_path).join("aethercore").join("aethercore.toml");
        if steam_toml.exists() {
            let content = std::fs::read_to_string(&steam_toml).unwrap_or_default();
            let re = regex::Regex::new(r#"(?m)^level\s*=\s*".*""#).unwrap();
            let updated = if re.is_match(&content) {
                re.replace(&content, format!("level = \"{}\"", lower)).to_string()
            } else {
                format!("{}\n\n[log]\nlevel = \"{}\"\nkeep_last_session = true\n", content, lower)
            };
            let _ = std::fs::write(&steam_toml, updated);
        }
    }

    crate::desk_log_info!("logs", "Set logging level to '{}' for Desk and DLL (aethercore.toml)", lower);
    Ok(format!("Logging level set to '{}' for Desk and DLL.", lower))
}

fn read_uco2_tail_lines(limit: usize) -> Vec<String> {
    let path = uco2_log_path();
    if !path.is_file() {
        return Vec::new();
    }
    if let Ok(content) = std::fs::read_to_string(&path) {
        let lines: Vec<String> = normalize_uco2_content(&content)
            .lines()
            .map(|s| s.to_string())
            .collect();
        if lines.len() <= limit {
            return lines;
        }
        return lines[lines.len() - limit..].to_vec();
    }
    Vec::new()
}

/// Normalizza il contenuto del log UCO2 lato AetherDesk (senza ricompilare
/// la DLL): timestamp ridotti a sola ora ("[2026-08-17 12:45:40.924]" ->
/// "[12:45:40.924]") e prefisso messaggi "[UCOnline2]" -> "[UCO2]".
fn normalize_uco2_content(content: &str) -> String {
    content
        .lines()
        .map(strip_date_from_timestamp)
        .map(|line| line.replace("[UCOnline2]", "[UCO2]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// "[YYYY-MM-DD HH:MM:SS.mmm] ..." -> "[HH:MM:SS.mmm] ..." (no-op se non matcha).
fn strip_date_from_timestamp(line: &str) -> String {
    let bytes = line.as_bytes();
    if line.len() > 24
        && bytes[0] == b'['
        && bytes[5] == b'-'
        && bytes[8] == b'-'
        && bytes[11] == b' '
        && bytes[24] == b']'
    {
        // La ']' originale (indice 24) viene saltata: la reinseriamo noi.
        return format!("[{}]{}", &line[12..24], &line[25..]);
    }
    line.to_string()
}

/// Chiave di ordinamento cronologico: toglie l'eventuale data "[YYYY-MM-DD "
/// dal prefisso timestamp, così righe con formati diversi (Desk full, UCO2
/// ridotto) si ordinano correttamente per orario.
fn sort_key(line: &str) -> String {
    let inside = line.split(']').next().unwrap_or("").trim_start_matches('[').trim();
    let b = inside.as_bytes();
    if inside.len() >= 11 && b[4] == b'-' && b[7] == b'-' && b[10] == b' ' {
        inside[11..].to_string()
    } else {
        inside.to_string()
    }
}

fn read_dll_tail_lines(app: &tauri::AppHandle, limit: usize) -> Vec<String> {
    let steam_path = crate::core::settings::SettingsManager::new(app).load().steam_path;
    let steam_path_buf = PathBuf::from(&steam_path);
    let desk_log_dir = crate::core::paths::LocalAppPaths::data_root().join("logs");
    let install_root = crate::core::paths::LocalAppPaths::install_root();

    for dir in [
        steam_path_buf.join("aethercore"),
        steam_path_buf.join("AetherDLL"),
        steam_path_buf.join("logs"),
        steam_path_buf.clone(),
        desk_log_dir.clone(),
        install_root.clone(),
    ] {
        let path = dir.join("main.log");
        if path.is_file() {
            if let Ok(mut file) = std::fs::File::open(&path) {
                let mut content = String::new();
                if let Ok(_) = std::io::Read::read_to_string(&mut file, &mut content) {
                    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
                    if lines.len() <= limit {
                        return lines;
                    } else {
                        return lines[lines.len() - limit..].to_vec();
                    }
                }
            }
        }
    }
    Vec::new()
}

fn clear_dll_log(app: &tauri::AppHandle) {
    let steam_path = crate::core::settings::SettingsManager::new(app).load().steam_path;
    let steam_path_buf = PathBuf::from(&steam_path);
    let desk_log_dir = crate::core::paths::LocalAppPaths::data_root().join("logs");
    let install_root = crate::core::paths::LocalAppPaths::install_root();

    for dir in [
        steam_path_buf.join("aethercore"),
        steam_path_buf.join("AetherDLL"),
        steam_path_buf.join("logs"),
        steam_path_buf.clone(),
        desk_log_dir.clone(),
        install_root.clone(),
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
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
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

/// Esportazione sincrona (dentro `spawn_blocking`): la creazione dello zip
/// e la copia dei file non devono bloccare il runtime async.
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
            if let Ok(_) = std::fs::copy(&src, stage_dir.join(timed(dest_name))) {
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
            let dest = stage_dir.join(timed(dest_name));
            if src.is_file() && !dest.exists() {
                if let Ok(_) = std::fs::copy(&src, &dest) {
                    copied += 1;
                }
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

/// Scarica un singolo documento di log (la sorgente selezionata nella vista)
/// nella cartella Downloads, con lo stesso nome temporizzato dello zip
/// (es. `desk.log_14-32-05.txt`). Per "both"/"all" crea il documento unico
/// `all.log_<ora>.txt` con le righe taggate, stessa procedura degli altri.
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
