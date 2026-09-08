//! Single source of truth for the Steam installation path.
//!
//! Every layer (settings persistence, Tauri commands, scanners, installers)
//! funnels through this module so "what counts as a valid Steam path" is
//! defined exactly once:
//!
//! * [`normalize_steam_path`] — pure string cleanup (trim, quotes, trailing
//!   separators). No I/O, safe to call on every save and every read.
//! * [`resolve_steam_path`] — normalize + filesystem validation. Returns the
//!   usable root or a typed [`SteamPathError`].
//! * [`validate_steam_path`] — command-layer guard with human-readable errors.
//! * [`detect_steam_installation`] / [`detect_steam_path`] — best-effort
//!   auto-detection (registry → running process → well-known locations).
//! * [`default_steam_path`] — default for fresh installs (detected, else legacy).
//!
//! Design rules: this module owns no Tauri types and no global state, so it
//! stays unit-testable in isolation and every consumer depends only on plain
//! functions (low coupling, high cohesion).

use std::path::{Path, PathBuf};

/// Historical install location. Kept as the last-resort fallback default so
/// fresh installs behave like previous releases when auto-detection finds
/// nothing. Also used to recognise a stale never-configured value that the
/// startup self-heal may replace with a detected installation.
pub const LEGACY_DEFAULT_STEAM_PATH: &str = r"C:\Program Files (x86)\Steam";

/// Sentinel file that must exist inside a valid Steam root.
pub const STEAM_EXE_FILE_NAME: &str = "steam.exe";

/// Why a configured Steam path cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamPathError {
    /// Nothing configured (empty after normalization).
    Empty,
    /// Normalized path does not exist.
    NotFound,
    /// Path exists but is not a directory.
    NotADirectory,
    /// Directory exists but contains no `steam.exe`.
    MissingSteamExe,
}

impl SteamPathError {
    /// Human-readable message for UI display. The `Empty` wording is kept
    /// byte-identical to the historical validation message.
    pub fn message(self, raw: &str) -> String {
        match self {
            SteamPathError::Empty => "Steam installation path is required".to_string(),
            SteamPathError::NotFound => format!(
                "Steam installation path was not found: {}. Check the path in Settings.",
                raw.trim()
            ),
            SteamPathError::NotADirectory => format!(
                "Steam installation path is not a directory: {}. Check the path in Settings.",
                raw.trim()
            ),
            SteamPathError::MissingSteamExe => format!(
                "steam.exe was not found in {}. Select the folder that contains steam.exe.",
                raw.trim()
            ),
        }
    }
}

/// Normalize a user-provided path: trim whitespace, strip one pair of
/// surrounding quotes (Explorer copy-paste), drop trailing separators.
/// A bare drive root (`D:\`) is preserved. Pure function, no I/O.
pub fn normalize_steam_path(raw: &str) -> String {
    let unquoted = {
        let trimmed = raw.trim();
        trimmed
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .or_else(|| {
                trimmed
                    .strip_prefix('\'')
                    .and_then(|inner| inner.strip_suffix('\''))
            })
            .unwrap_or(trimmed)
            .trim()
    };
    if unquoted.is_empty() {
        return String::new();
    }
    let stripped = unquoted.trim_end_matches(['/', '\\']);
    if stripped.is_empty() {
        // Input was only separators (e.g. `\`): keep it verbatim so the
        // resolver reports a precise `NotFound` instead of `Empty`.
        return unquoted.to_string();
    }
    if stripped.len() == 2 && stripped.as_bytes()[1] == b':' {
        // `D:\` trimmed down to `D:`, which is not a usable directory.
        return format!("{stripped}\\");
    }
    stripped.to_string()
}

/// True when `path` is an existing directory containing `steam.exe`.
pub fn is_valid_steam_dir(path: &Path) -> bool {
    path.is_dir() && path.join(STEAM_EXE_FILE_NAME).is_file()
}

/// Normalize + validate. Returns the usable installation root on success.
pub fn resolve_steam_path(raw: &str) -> Result<PathBuf, SteamPathError> {
    let normalized = normalize_steam_path(raw);
    if normalized.is_empty() {
        return Err(SteamPathError::Empty);
    }
    let path = PathBuf::from(&normalized);
    if !path.exists() {
        return Err(SteamPathError::NotFound);
    }
    if !path.is_dir() {
        return Err(SteamPathError::NotADirectory);
    }
    if !path.join(STEAM_EXE_FILE_NAME).is_file() {
        return Err(SteamPathError::MissingSteamExe);
    }
    Ok(path)
}

/// Canonical command-layer guard: validates like [`resolve_steam_path`] and
/// converts the typed error into a display-ready message.
pub fn validate_steam_path(raw: &str) -> Result<(), String> {
    resolve_steam_path(raw)
        .map(|_| ())
        .map_err(|error| error.message(raw))
}

/// True when `raw` still holds the historical never-configured default
/// (case- and separator-insensitive). Used by the startup self-heal to decide
/// whether an invalid value may be replaced by auto-detection: an explicit
/// custom path is always respected, even when temporarily unreachable.
pub fn is_legacy_default_path(raw: &str) -> bool {
    fn canonical(path: &str) -> String {
        normalize_steam_path(path).replace('/', "\\").to_lowercase()
    }
    canonical(raw) == canonical(LEGACY_DEFAULT_STEAM_PATH)
}

/// Where an auto-detected Steam installation came from (diagnostics/startup log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamPathSource {
    Registry,
    RunningProcess,
    WellKnownLocation,
}

impl SteamPathSource {
    pub fn as_log_label(self) -> &'static str {
        match self {
            SteamPathSource::Registry => "registry",
            SteamPathSource::RunningProcess => "running-process",
            SteamPathSource::WellKnownLocation => "well-known-location",
        }
    }
}

/// Best-effort Steam auto-detection: registry → running `steam.exe` →
/// well-known locations. Returns the first directory that actually contains
/// `steam.exe`. Pure discovery — never writes anything.
pub fn detect_steam_installation() -> Option<(PathBuf, SteamPathSource)> {
    if let Some(path) = steam_path_from_registry() {
        if is_valid_steam_dir(&path) {
            return Some((path, SteamPathSource::Registry));
        }
    }
    if let Some(path) = steam_path_from_running_process() {
        if is_valid_steam_dir(&path) {
            return Some((path, SteamPathSource::RunningProcess));
        }
    }
    for candidate in well_known_candidates() {
        if is_valid_steam_dir(&candidate) {
            return Some((candidate, SteamPathSource::WellKnownLocation));
        }
    }
    None
}

/// Convenience wrapper when the source label is not needed.
pub fn detect_steam_path() -> Option<PathBuf> {
    detect_steam_installation().map(|(path, _)| path)
}

/// Default for fresh installs: detected location, else the historical path.
pub fn default_steam_path() -> String {
    detect_steam_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| LEGACY_DEFAULT_STEAM_PATH.to_string())
}

/// `InstallPath` from the Valve registry keys. Both the native and the
/// WoW6432Node views are probed, HKCU first (per-user installs) then HKLM
/// (machine-wide installs).
#[cfg(target_os = "windows")]
fn steam_path_from_registry() -> Option<PathBuf> {
    const SUBKEYS: [&str; 2] = [
        r"SOFTWARE\Valve\Steam",
        r"SOFTWARE\Wow6432Node\Valve\Steam",
    ];
    for hive in [
        winreg::enums::HKEY_CURRENT_USER,
        winreg::enums::HKEY_LOCAL_MACHINE,
    ] {
        let root = winreg::RegKey::predef(hive);
        for subkey in SUBKEYS {
            let install_path: Result<String, _> = root
                .open_subkey(subkey)
                .and_then(|key| key.get_value::<String, _>("InstallPath"));
            if let Ok(install_path) = install_path {
                let normalized = normalize_steam_path(&install_path);
                if !normalized.is_empty() {
                    return Some(PathBuf::from(normalized));
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn steam_path_from_registry() -> Option<PathBuf> {
    None
}

/// Parent directory of a running `steam.exe`, if one is visible to this process.
fn steam_path_from_running_process() -> Option<PathBuf> {
    // Lightweight snapshot: process list only (no full system refresh).
    let mut system = sysinfo::System::new();
    system.refresh_processes();
    system.processes().values().find_map(|process| {
        let name = process.name().to_lowercase();
        if name != "steam.exe" && name != "steam" {
            return None;
        }
        process.exe()?.parent().map(Path::to_path_buf)
    })
}

/// Historical and common install locations. Environment-driven roots first
/// (they follow the OS), then fixed drive guesses for custom installs.
fn well_known_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for env_key in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
        if let Ok(root) = std::env::var(env_key) {
            candidates.push(PathBuf::from(root).join("Steam"));
        }
    }
    candidates.push(PathBuf::from(LEGACY_DEFAULT_STEAM_PATH));
    for drive in ["D", "E", "F", "G"] {
        candidates.push(PathBuf::from(format!(r"{drive}:\Steam")));
        candidates.push(PathBuf::from(format!(r"{drive}:\Games\Steam")));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Creates `<tmp>/Steam/steam.exe` (empty sentinel file) and returns the tmp dir.
    fn fake_installation() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path().join("Steam");
        fs::create_dir_all(&root).expect("fake steam dir");
        fs::write(root.join(STEAM_EXE_FILE_NAME), b"fake").expect("fake steam.exe");
        tmp
    }

    #[test]
    fn normalize_trims_whitespace_and_trailing_separators() {
        assert_eq!(normalize_steam_path("  D:\\Steam  "), r"D:\Steam");
        assert_eq!(normalize_steam_path("D:\\Steam\\"), r"D:\Steam");
        assert_eq!(normalize_steam_path("D:\\Steam///"), r"D:\Steam");
        assert_eq!(normalize_steam_path(""), "");
        assert_eq!(normalize_steam_path("   "), "");
    }

    #[test]
    fn normalize_strips_surrounding_quotes_but_keeps_inner_spaces() {
        assert_eq!(normalize_steam_path("\"D:\\My Games\\Steam\""), r"D:\My Games\Steam");
        assert_eq!(normalize_steam_path("'D:\\Steam' "), r"D:\Steam");
        assert_eq!(normalize_steam_path("D:\\My Games\\Steam"), r"D:\My Games\Steam");
    }

    #[test]
    fn normalize_preserves_drive_roots() {
        assert_eq!(normalize_steam_path("D:\\"), r"D:\");
        assert_eq!(normalize_steam_path("D:/"), r"D:\");
    }

    #[test]
    fn resolve_accepts_only_dirs_containing_steam_exe() {
        let tmp = fake_installation();
        let root = tmp.path().join("Steam");

        let resolved = resolve_steam_path(&root.display().to_string()).expect("valid install");
        assert_eq!(resolved, root);

        // Quoted + padded input resolves to the same root.
        let padded = format!("  \"{}\"  ", root.display());
        assert_eq!(resolve_steam_path(&padded).expect("padded"), root);

        assert_eq!(resolve_steam_path(""), Err(SteamPathError::Empty));
        assert_eq!(resolve_steam_path("   "), Err(SteamPathError::Empty));
        assert_eq!(
            resolve_steam_path(&tmp.path().join("missing").display().to_string()),
            Err(SteamPathError::NotFound)
        );

        let plain_dir = tmp.path().join("plain");
        fs::create_dir_all(&plain_dir).expect("plain dir");
        assert_eq!(
            resolve_steam_path(&plain_dir.display().to_string()),
            Err(SteamPathError::MissingSteamExe)
        );

        let file = tmp.path().join("file.txt");
        fs::write(&file, b"x").expect("file");
        assert_eq!(
            resolve_steam_path(&file.display().to_string()),
            Err(SteamPathError::NotADirectory)
        );
    }

    #[test]
    fn validate_keeps_historical_empty_message() {
        assert_eq!(
            validate_steam_path("   ").unwrap_err(),
            "Steam installation path is required"
        );
        assert!(validate_steam_path("Z:\\definitely-not-steam").is_err());
    }

    #[test]
    fn legacy_default_matches_case_and_separator_insensitively() {
        assert!(is_legacy_default_path(r"C:\Program Files (x86)\Steam"));
        assert!(is_legacy_default_path(r"  c:\program files (x86)\steam\  "));
        assert!(is_legacy_default_path("C:/Program Files (x86)/Steam"));
        assert!(!is_legacy_default_path(r"D:\Steam"));
        assert!(!is_legacy_default_path(""));
    }

    #[test]
    fn detection_never_returns_an_invalid_dir() {
        // Environment-dependent (may or may not find Steam), but any result
        // must always pass validation.
        if let Some((path, _)) = detect_steam_installation() {
            assert!(is_valid_steam_dir(&path));
        }
    }

    #[test]
    fn default_is_detected_or_legacy() {
        let default = default_steam_path();
        assert!(!default.trim().is_empty());
        if detect_steam_path().is_none() {
            assert_eq!(default, LEGACY_DEFAULT_STEAM_PATH);
        }
    }
}
