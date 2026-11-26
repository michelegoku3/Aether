//! Achievement schema inspection, state reconciliation, and transactional editor.
//!
//! Operates on Steam's binary KV1 caches (`UserGameStatsSchema_<appid>.bin` and
//! `UserGameStats_<account>_<appid>.bin`) alongside AetherDLL's backup mirrors
//! under `<AetherData>/backup/<appid>/achievements/`.

pub mod kv1;
pub mod schema;
pub mod state;
