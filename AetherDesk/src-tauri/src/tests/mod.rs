// Test suite dedicata — tutti i test unitari sono qui, nessun `#[cfg(test)]` nei file sorgente.
// Ogni file contiene i test per un modulo specifico; importano via `crate::`.
pub mod aliases_tests;
pub mod crack_locate_tests;
pub mod custom_css_tests;
pub mod dll_version_tests;
pub mod github_updater_tests;
pub mod manifest_pins_tests;
pub mod normalize_tests;
pub mod online_bundle_tests;
pub mod online_deploy_config_tests;
pub mod online_detect_tests;
pub mod online_serde_tests;
pub mod service_tests;
pub mod store_items_tests;
pub mod store_suggest_tests;
pub mod versioning_snapshot_tests;
