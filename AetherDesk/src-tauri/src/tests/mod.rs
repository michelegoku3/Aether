// Test suite dedicata — tutti i test unitari sono qui, nessun `#[cfg(test)]` nei file sorgente.
// Ogni file contiene i test per un modulo specifico; importano via `crate::`.
pub mod aliases_tests;
pub mod normalize_tests;
pub mod store_items_tests;
pub mod dll_version_tests;
pub mod service_tests;
