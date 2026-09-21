//! Steam-path guard for the command layer.
//!
//! Single source of truth lives in [`crate::steam::resolve`]; this module only
//! re-exports the canonical guard (normalize + exists + is a directory +
//! contains `steam.exe`, with human-readable errors).
//!
//! Nessun comando riceve più `steam_path` dal frontend: i comandi passano da
//! [`crate::commands::command_steam_path`] (validazione rigorosa, errore
//! leggibile) o [`crate::commands::configured_steam_path`] (degrado gentile),
//! che usano questa guardia al loro interno. Chiama direttamente
//! `validate_steam_path` solo il codice che ha già un percorso in mano — per
//! esempio i comandi presence, che lo leggono dalle impostazioni, e
//! `check_steam_path`, che valida un percorso CANDIDATO prima del save
//! (docs/shared_contracts.md §8).

pub use crate::steam::resolve::validate_steam_path;
