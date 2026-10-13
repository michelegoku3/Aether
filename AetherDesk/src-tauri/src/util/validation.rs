//! Steam-path guard for the command layer.
//!
//! Single source of truth lives in [`crate::steam::resolve`]; this module only
//! re-exports the canonical guard (normalize + exists + is a directory +
//! contains `steam.exe`, with human-readable errors).
//!
//! No command receives `steam_path` from the frontend anymore: commands go
//! through [`crate::commands::command_steam_path`] (strict validation, readable
//! error) or [`crate::commands::configured_steam_path`] (graceful degradation),
//! which use this guard internally. Call `validate_steam_path` directly only
//! from code that already holds a path — e.g. the presence commands, which
//! read it from settings, and `check_steam_path`, which validates a CANDIDATE
//! path before save (docs/shared_contracts.md §8).

pub use crate::steam::resolve::validate_steam_path;
