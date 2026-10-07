//! Application use cases.
//!
//! This layer coordinates domain services and infrastructure adapters. Tauri
//! commands delegate here and remain responsible only for the IPC boundary.
//! Application modules must not depend on `commands`.

pub mod store;
