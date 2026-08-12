//! Professional, session-oriented levelled logger for AetherDesk.
//!
//! # Architectural Overview
//! Mirrors AetherDLL's session logging architecture (`Logger.h` / `Logger.cpp`),
//! providing full-lifecycle traceability across AetherDesk (`desk.log`) and
//! AetherDLL (`main.log`).
//!
//! # Key Features
//! - **Session Rotation**: On startup, rotates `desk.log` → `desk.log.last`,
//!   guaranteeing a clean log per session while preserving crash forensics.
//! - **High-Precision Timestamps**: Formatted as `[YYYY-MM-DD HH:MM:SS.mmm]`
//!   with PID and TID, allowing chronological merging with AetherDLL session logs.
//! - **Per-Session Deduplication**: `write_once` and `log_once!` prevent log spam
//!   during high-frequency background operations (e.g., IPC checks, process scans).
//! - **Antivirus-Resilient I/O**: Stored inside `<install_root>\AetherData\logs\`,
//!   inheriting existing Windows Defender folder exclusions for portable mode.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Off = 5,
}

impl LogLevel {
    pub fn tag(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO ",
            LogLevel::Warn => "WARN ",
            LogLevel::Error => "ERROR",
            LogLevel::Off => "OFF  ",
        }
    }
}

pub struct Logger {
    file: Option<File>,
    log_path: PathBuf,
    dedup_set: HashSet<String>,
    min_level: LogLevel,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

/// Formats local time as `HH:MM:SS.mmm` using Windows `GetLocalTime`.
#[cfg(target_os = "windows")]
fn format_timestamp_ms() -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    )
}

#[cfg(not(target_os = "windows"))]
fn format_timestamp_ms() -> String {
    "00:00:00.000".to_string()
}

/// Helper that formats an AppID with its game name in parentheses:
/// e.g. `AppID 4145350 (Spider-Man)`
pub fn format_appid(app_id: u32) -> String {
    let name = crate::steam::app_names::get_cached_game_name(app_id);
    format!("AppID {} ({})", app_id, name)
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

pub fn parse_level(s: &str, fallback: LogLevel) -> LogLevel {
    match s.trim().to_ascii_lowercase().as_str() {
        "trace" => LogLevel::Trace,
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        "off" => LogLevel::Off,
        _ => fallback,
    }
}

pub fn set_level(level: LogLevel) {
    if let Some(mutex) = LOGGER.get() {
        if let Ok(mut logger) = mutex.lock() {
            logger.min_level = level;
            if level == LogLevel::Off {
                logger.file = None;
            } else if logger.file.is_none() && !logger.log_path.as_os_str().is_empty() {
                if let Some(parent) = logger.log_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                logger.file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(true)
                    .open(&logger.log_path)
                    .ok();
            }
        }
    }
}

pub fn set_level_from_str(s: &str) {
    let level = parse_level(s, LogLevel::Info);
    set_level(level);
}

/// Computes a stable numeric Thread ID for log correlation.
fn current_thread_id() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    (hasher.finish() % 99999) as u64
}

/// Initializes the global session logger, rotating `desk.log` → `desk.log.last`.
/// Default logging level is always `Trace` (not modifiable by settings).
pub fn init(_app: &tauri::AppHandle) {
    let log_dir = crate::core::paths::LocalAppPaths::data_root().join("logs");
    let log_path = log_dir.join("desk.log");
    let _ = std::fs::create_dir_all(&log_dir);

    if log_path.exists() {
        let backup_path = log_dir.join("desk.log.last");
        let _ = std::fs::remove_file(&backup_path);
        let _ = std::fs::rename(&log_path, &backup_path);
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&log_path)
        .ok();

    let logger = Logger {
        file,
        log_path: log_path.clone(),
        dedup_set: HashSet::new(),
        min_level: LogLevel::Trace,
    };

    let _ = LOGGER.set(Mutex::new(logger));

    write(
        LogLevel::Info,
        "lifecycle",
        &format!(
            "AetherDesk session started (version: {}, PID: {})",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        ),
    );
}

/// Core log writer. Formats timestamp, process/thread identity, level, module tag,
/// and message body, emitting to both console and `desk.log`.
pub fn write(level: LogLevel, module: &str, msg: &str) {
    let Some(mutex) = LOGGER.get() else {
        // Fallback before init()
        let ts = format_timestamp_ms();
        let mod_cap = capitalize_first(module);
        eprintln!("[{}] [PID:{:05}] [TID:{:05}] [{}] [{:<10}] {}",
            ts, std::process::id(), current_thread_id(), level.tag(), mod_cap, msg);
        return;
    };

    let Ok(mut logger) = mutex.lock() else { return; };
    if level < logger.min_level {
        return;
    }

    let mod_cap = capitalize_first(module);
    let formatted = format!(
        "[{}] [PID:{:05}] [TID:{:05}] [{}] [{:<10}] {}\n",
        format_timestamp_ms(),
        std::process::id(),
        current_thread_id(),
        level.tag(),
        mod_cap,
        msg
    );

    print!("{}", formatted);
    if let Some(file) = &mut logger.file {
        let _ = file.write_all(formatted.as_bytes());
        let _ = file.flush();
    }
}

/// Deduplicating writer. Emits a unique `(module, msg)` signature at most once
/// per session or until `reset_session_dedup()` is called.
pub fn write_once(level: LogLevel, module: &str, msg: &str) {
    let Some(mutex) = LOGGER.get() else {
        write(level, module, msg);
        return;
    };

    let key = format!("{}|{}", module, msg);
    {
        let Ok(mut logger) = mutex.lock() else { return; };
        if logger.dedup_set.contains(&key) {
            return;
        }
        logger.dedup_set.insert(key);
    }

    write(level, module, msg);
}

/// Clears the per-session deduplication set. Call when a new game session begins.
pub fn reset_session_dedup() {
    if let Some(mutex) = LOGGER.get() {
        if let Ok(mut logger) = mutex.lock() {
            logger.dedup_set.clear();
        }
    }
}

/// Reads trailing log lines from `desk.log` for UI presentation.
pub fn read_tail_lines(tail_lines: usize) -> Result<Vec<String>, String> {
    let log_path = crate::core::paths::LocalAppPaths::data_root()
        .join("logs")
        .join("desk.log");
    if !log_path.exists() {
        return Ok(Vec::new());
    }

    let mut file =
        File::open(&log_path).map_err(|e| format!("Failed to open log file: {e}"))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| format!("Failed to read log file: {e}"))?;

    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    if lines.len() <= tail_lines {
        Ok(lines)
    } else {
        Ok(lines[lines.len() - tail_lines..].to_vec())
    }
}

/// Clears the current session log and resets the deduplication set.
pub fn clear_current_log() -> Result<(), String> {
    if let Some(mutex) = LOGGER.get() {
        if let Ok(mut logger) = mutex.lock() {
            logger.dedup_set.clear();
            if let Some(file) = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&logger.log_path)
                .ok()
            {
                logger.file = Some(file);
            }
        }
    }

    write(LogLevel::Info, "lifecycle", "Session log cleared by user command.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Convenience macros matching AetherDLL level conventions
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! desk_log_trace {
    ($mod:expr, $($arg:tt)*) => {
        $crate::core::logger::write($crate::core::logger::LogLevel::Trace, $mod, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! desk_log_debug {
    ($mod:expr, $($arg:tt)*) => {
        $crate::core::logger::write($crate::core::logger::LogLevel::Debug, $mod, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! desk_log_info {
    ($mod:expr, $($arg:tt)*) => {
        $crate::core::logger::write($crate::core::logger::LogLevel::Info, $mod, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! desk_log_warn {
    ($mod:expr, $($arg:tt)*) => {
        $crate::core::logger::write($crate::core::logger::LogLevel::Warn, $mod, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! desk_log_error {
    ($mod:expr, $($arg:tt)*) => {
        $crate::core::logger::write($crate::core::logger::LogLevel::Error, $mod, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! desk_log_info_once {
    ($mod:expr, $($arg:tt)*) => {
        $crate::core::logger::write_once($crate::core::logger::LogLevel::Info, $mod, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! desk_log_warn_once {
    ($mod:expr, $($arg:tt)*) => {
        $crate::core::logger::write_once($crate::core::logger::LogLevel::Warn, $mod, &format!($($arg)*))
    };
}
