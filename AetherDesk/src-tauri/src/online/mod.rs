//! Engine UCOnline2 di AetherDesk: rilevamento, configurazione, deploy e
//! rollback di UCOnline2 su un gioco installato.
//!
//! Regole architetturali:
//!   * questo modulo è PURO — nessuna dipendenza da Tauri; riceve `PathBuf`
//!     e struct, restituisce struct `Serialize`;
//!   * i comandi Tauri (`commands/online.rs`) fanno solo da collante;
//!   * ogni responsabilità vive nel proprio file (detect, config, deploy,
//!     revert, state) — niente file tuttofare.

pub mod bundle;
pub mod config;
pub mod deploy;
pub mod detect;
pub mod engine;
pub mod foreign;
pub mod preferences;
pub mod revert;
pub mod state;
pub mod steamstub;
pub mod types;
