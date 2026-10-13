//! Shared contract constants across modules/tools.
//!
//! The single place where the backup/rename suffixes live, used by both the
//! external tools and the online engine: a change propagates by itself
//! (DRY) and tests rely on the same values used in production.

/// Backup suffix for Steamless-processed executables
/// (e.g. `game.exe.steamstub.bak` — the Steamless tool's own naming).
pub const STEAMLESS_BACKUP_SUFFIX: &str = ".steamstub.bak";

/// Suffix of the Steamless output before it replaces the original
/// (e.g. `game.exe.unpacked.exe`).
pub const STEAMLESS_UNPACKED_SUFFIX: &str = ".unpacked.exe";

/// Suffix with which UCOnline2 (patch.bat) reversibly neutralizes competing
/// emulators (SteamFix/OFME from online-fix.me and their proxies).
pub const UCO_DISABLED_SUFFIX: &str = ".uco-disabled";

/// Threshold (bytes) below which a generic proxy DLL (`version.dll`, `dxgi.dll`,
/// ...) counts as part of a competing emulator rather than a legitimate game
/// file. Mirrors the UCOnline2 patch.bat threshold (300 KiB).
pub const UCO_PROXY_MAX_BYTES: u64 = 307_200;

/// True when a file name is a Steamless backup.
pub fn is_steamless_backup_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(STEAMLESS_BACKUP_SUFFIX)
}

/// True when a file name is a not-yet-applied Steamless output.
pub fn is_steamless_unpacked_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(STEAMLESS_UNPACKED_SUFFIX)
}
