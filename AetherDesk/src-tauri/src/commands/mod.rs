//! Tauri command modules + backend-resolved request context.
//!
//! ## The Steam path is not a client parameter
//!
//! The Steam path is resolved by the backend (single source of truth: the
//! settings), not passed by the frontend. That keeps the config read and the
//! **DPAPI keystore decryption** out of every call, prevents a client from
//! passing a path different from the configured one, and avoids re-validating
//! it in every command.
//!
//! Commands use [`command_steam_path`] (fails with a readable user message)
//! or [`configured_steam_path`] (degrades gracefully when the path is missing
//! or Steam is unreachable).

// ---------------------------------------------------------------------------
// IPC boundary lint policy (deliberate, motivated exceptions).
//
// `clippy::needless_pass_by_value`: command params are resolved by the
// `generate_handler!` macro through the `tauri::ipc::CommandArg` trait, which
// is implemented only for **by value** types (`AppHandle`, `String`, ...): the
// `&AppHandle`/`&str` signatures suggested by the lint do not compile. By value
// here is a framework constraint, not a style choice.
//
// `clippy::unnecessary_wraps`: the `Result<T, String>` signature is the
// intentional IPC contract: every command exposes the same error surface to
// the frontend until the migration to `DeskError` is complete (policy in
// Cargo.toml). Removing `Result` from individual commands "that cannot fail
// today" would make the contract inhomogeneous and block adding error paths
// without a breaking change for the UI.
//
// Non-command internals stay subject to both lints.
// ---------------------------------------------------------------------------
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unnecessary_wraps)]

pub mod aether_desk;
pub mod aether_dll;
pub mod antivirus;
pub mod crack;
pub mod custom_css;
pub mod fs;
pub mod game_info;
pub mod home_links;
pub mod library;
pub mod local;
pub mod logs;
pub mod manifests;
pub mod monitor;
pub mod online;
pub mod settings;
pub mod steam;
pub mod steamless;
pub mod store;
pub mod versioning;
pub mod window;
pub mod workshop;

use crate::core::settings::require_steam_path;
use crate::util::validation::validate_steam_path;

/// Configured Steam path, normalized and filesystem-validated.
///
/// Fails with a readable error when the path is missing (`require_steam_path`)
/// and enforces strict validation when present (`validate_steam_path`). The
/// client neither reads nor supplies the path.
///
/// Use this when the command **cannot** work without a valid Steam path
/// (install/uninstall, manifest writes, downloads).
pub fn command_steam_path(app: &tauri::AppHandle) -> Result<String, String> {
    let steam_path = require_steam_path(app)?;
    validate_steam_path(&steam_path)?;
    Ok(steam_path)
}

/// Configured Steam path **without** filesystem validation.
///
/// For commands that must degrade gracefully instead of returning an error:
/// DLL version reads, update checks, residue probes. With `None`
/// (unconfigured) or an unreachable path the caller answers "N/A" / 0 / no
/// updates.
pub fn configured_steam_path(app: &tauri::AppHandle) -> Option<String> {
    require_steam_path(app).ok()
}
