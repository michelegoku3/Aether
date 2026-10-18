//! Test suite dedicated to application modules.
//!
//! ## Real convention (updated: this comment used to describe it wrongly)
//!
//! Two styles coexist in the crate, both legitimate:
//!
//! 1. **Cross-cutting / contract tests** → here in `src/tests/`, one file per
//!    area (`ipc_contract_tests.rs`, `manifest_pins_tests.rs`, …). These are the
//!    tests spanning several modules or verifying a contract toward the outside
//!    (frontend, AetherDLL, on-disk format). They import via `crate::`.
//! 2. **Inline unit tests** → `#[cfg(test)] mod tests` at the bottom of the file
//!    implementing the logic. 13 files / 46 tests (e.g. `steam/resolve.rs`,
//!    `core/backup.rs`, `versioning/sources/*`): they live next to the code they
//!    cover because they test private functions or local details.
//!
//! Total: 389 tests (`cargo test`, counting `#[test]` attributes). The rule of thumb for placing a new test:
//!
//! - touches **more than one module**, or verifies something another part of the
//!   system (frontend, DLL, files on disk) must honor → `src/tests/`;
//! - tests a **private** function or a local invariant → inline in the file.
//!
//! Historical note: this file used to claim "no `#[cfg(test)]` in source
//! files", disproved by the code. Moving the 46 inline tests here would be churn
//! without benefit; the dual convention is now documented as it really is.

pub mod aliases_tests;
pub mod bridge_contract_tests;
pub mod crack_locate_tests;
pub mod custom_css_tests;
pub mod desk_error_tests;
pub mod dll_install_tests;
pub mod dll_version_tests;
pub mod github_updater_tests;
pub mod home_links_tests;
pub mod hubcap_monitor_tests;
pub mod hubcap_quota_tests;
pub mod ipc_contract_tests;
pub mod local_file_classification_tests;
pub mod local_lua_name_tests;
pub mod lua_build_validation_tests;
pub mod luatools_provider_tests;
pub mod manifest_contract_tests;
pub mod manifest_identity_tests;
pub mod manifest_package_tests;
pub mod manifest_pins_tests;
pub mod manifest_source_tests;
pub mod normalize_tests;
pub mod online_bundle_tests;
pub mod online_deploy_config_tests;
pub mod online_detect_tests;
pub mod online_foreign_tests;
pub mod online_serde_tests;
pub mod resolver_tests;
pub mod service_tests;
pub mod settings_usage_cache_tests;
pub mod steam_library_install_state_tests;
pub mod steam_library_issue_tests;
pub mod store_items_tests;
pub mod store_suggest_tests;
pub mod tauri_config_tests;
pub mod versioning_snapshot_tests;
