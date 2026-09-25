//! Test suite dedicata ai moduli applicativi.
//!
//! ## Convenzione reale (aggiornata: prima questo commento la descriveva male)
//!
//! Nel crate convivono **due** stili, entrambi legittimi:
//!
//! 1. **Test trasversali / di contratto** → qui in `src/tests/`, un file per
//!    area (`ipc_contract_tests.rs`, `manifest_pins_tests.rs`, …). Sono i test
//!    che attraversano più moduli o che verificano un contratto verso l'esterno
//!    (frontend, AetherDLL, formato su disco). Importano via `crate::`.
//! 2. **Test unitari inline** → `#[cfg(test)] mod tests` in fondo al file che
//!    implementa la logica. Sono 13 file / 46 test (es. `steam/resolve.rs`,
//!    `core/backup.rs`, `versioning/sources/*`): vivono accanto al codice che
//!    coprono perché ne testano funzioni private o dettagli locali.
//!
//! Totale: 240 test (`cargo test`). La regola pratica per scegliere dove
//! mettere un test nuovo:
//!
//! - tocca **più di un modulo**, o verifica qualcosa che un'altra parte del
//!   sistema (frontend, DLL, file su disco) deve rispettare → `src/tests/`;
//! - testa una funzione **privata** o un invariante locale → inline nel file.
//!
//! Nota storica: questo file affermava "nessun `#[cfg(test)]` nei file
//! sorgente", smentito dal codice. Spostare i 46 test inline qui sarebbe churn
//! senza beneficio; la convenzione doppia è ora documentata com'è davvero.

pub mod aliases_tests;
pub mod bridge_contract_tests;
pub mod crack_locate_tests;
pub mod custom_css_tests;
pub mod dll_version_tests;
pub mod dll_install_tests;
pub mod github_updater_tests;
pub mod home_links_tests;
pub mod hubcap_monitor_tests;
pub mod hubcap_quota_tests;
pub mod ipc_contract_tests;
pub mod manifest_identity_tests;
pub mod manifest_package_tests;
pub mod manifest_pins_tests;
pub mod manifest_source_tests;
pub mod resolver_tests;
pub mod luatools_provider_tests;
pub mod lua_build_validation_tests;
pub mod local_file_classification_tests;
pub mod local_lua_name_tests;
pub mod normalize_tests;
pub mod online_bundle_tests;
pub mod desk_error_tests;
pub mod online_deploy_config_tests;
pub mod online_detect_tests;
pub mod online_foreign_tests;
pub mod online_serde_tests;
pub mod service_tests;
pub mod settings_usage_cache_tests;
pub mod steam_library_issue_tests;
pub mod store_items_tests;
pub mod store_suggest_tests;
pub mod versioning_snapshot_tests;
