//! AetherDesk's UCOnline2 engine: detection, configuration, deploy and
//! rollback of UCOnline2 onto an installed game.
//!
//! Architecture rules:
//!   * this module is PURE — no Tauri dependency; takes `PathBuf`
//!     and structs, returns `Serialize` structs;
//!   * the Tauri commands (`commands/online.rs`) only glue things together;
//!   * every responsibility lives in its own file (detect, config, deploy,
//!     revert, state) — no catch-all files.

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
