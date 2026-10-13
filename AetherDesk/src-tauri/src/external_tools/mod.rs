//! Building blocks shared by the external tools integrated in AetherDesk
//! (Steamless, UCOnline2, ...).
//!
//! Each tool keeps its specific logic (runner for Steamless, deployer for
//! UCOnline2) in its own module; only what is truly identical between them
//! lives here: bundle location/installation, filesystem utilities and contract
//! constants. Rule: a piece needed by two tools lives here; needed by one
//! tool only, it lives in that tool's module.

pub mod bundle;
pub mod constants;
pub mod fs;
