//! Steam-path guard shared by every Tauri command that receives `steam_path`.
//!
//! Single source of truth lives in [`crate::steam::resolve`]; this module only
//! re-exports the canonical guard so existing call sites keep working
//! unchanged while transparently gaining full filesystem validation
//! (previously only a non-empty check).

pub use crate::steam::resolve::validate_steam_path;
