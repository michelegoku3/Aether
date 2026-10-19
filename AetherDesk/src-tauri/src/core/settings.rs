//! Settings facade: data model (`settings_model`) plus persistence and
//! transactions (`settings_repository`).
//!
//! The re-exports below preserve the historical paths, so
//! `crate::core::settings::AppSettings`,
//! `crate::core::settings::SettingsManager`, `merge_settings`, the
//! `normalize_*` helpers and friends all keep working untouched.
#[path = "settings_model.rs"]
mod settings_model;
#[path = "settings_repository.rs"]
mod settings_repository;

pub use settings_model::*;
pub use settings_repository::*;
