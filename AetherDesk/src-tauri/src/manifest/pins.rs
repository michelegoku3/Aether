//! Facade for parsing and editing manifest pins in managed Lua files.
//!
//! Public callers keep using `manifest::pins::*`. Internally, responsibilities
//! are split into model, parser, read-only document policy, in-memory editor
//! and this compare-and-write repository facade.
//!
//! Invariants:
//! - Lua is parsed as text and is never executed;
//! - manifest GIDs accepted here are decimal `u64`, matching AetherDLL;
//! - edits preserve the total number of `setManifestid` rows;
//! - malformed input may be inspected and repaired, but executable malformed
//!   output is never accepted;
//! - writes fail when the file changed after it was read.

mod document;
mod editor;
mod line_edit;
mod model;
mod parser;

pub use model::{
    pins_from_rows, ApplyBuildResult, DepotManifestPin, LuaManifestEdit, LuaManifestRow,
    LuaManifestRowIssue,
};
#[cfg(test)]
pub use model::ManifestGidProblem;

use std::fs;
use std::path::PathBuf;

/// Malformed rows that currently execute and therefore break the whole Lua.
/// Exposed only to the crate's characterization tests; production callers use
/// `first_active_issue` through the facade.
#[cfg(test)]
pub fn active_invalid_manifest_rows(content: &str) -> Vec<LuaManifestRowIssue> {
    document::active_invalid_rows(content)
}

/// Every malformed `setManifestid` row, including commented rows that would
/// become dangerous if updates were enabled later.
pub fn invalid_manifest_rows(content: &str) -> Vec<LuaManifestRowIssue> {
    parser::invalid_rows(content)
}

/// Stateful filesystem facade. Parsing and transformations are delegated to
/// pure/in-memory modules; this type owns path resolution and compare-and-write.
pub struct LuaManifestPins {
    lua_path: PathBuf,
}

impl LuaManifestPins {
    pub fn new(steam_path: impl Into<PathBuf>, root_app_id: u32) -> Self {
        let normalized =
            crate::steam::resolve::normalize_steam_path(&steam_path.into().to_string_lossy());
        Self {
            lua_path: PathBuf::from(normalized)
                .join("config")
                .join("stplug-in")
                .join(format!("{}.lua", root_app_id)),
        }
    }

    pub fn lua_path(&self) -> &std::path::Path {
        &self.lua_path
    }

    pub fn path_exists(&self) -> bool {
        self.lua_path.exists()
    }

    pub fn apply_build_pins(&self, pins: &[DepotManifestPin]) -> Result<ApplyBuildResult, String> {
        let content = self.read_lua()?;
        let current_count = parser::parse_pins(&content).len();
        let (next_content, result) = editor::apply_build_pins(&content, pins)?;
        self.write_lua(&content, &next_content)?;
        crate::desk_log_info!(
            "manifest",
            "Lua manifest {}: apply_build_pins completed -> {} pin(s) applied, {} depot(s) absent from the supplied snapshot and safely left unchanged",
            self.lua_path.display(),
            result.applied_pins,
            current_count - result.applied_pins
        );
        Ok(result)
    }

    pub fn realign_commented_pins(&self, pins: &[DepotManifestPin]) -> Result<usize, String> {
        let content = self.read_lua()?;
        let (next_content, realigned) = editor::realign_commented_pins(&content, pins)?;
        if let Some(next_content) = next_content {
            self.write_lua(&content, &next_content)?;
        }
        Ok(realigned)
    }

    pub fn rows_from_content(content: &str) -> Vec<LuaManifestRow> {
        document::rows_from_content(content)
    }

    pub fn rows_for_manifest_sync(content: &str) -> Vec<LuaManifestRow> {
        document::rows_for_manifest_sync(content)
    }

    pub fn validate_content(content: &str) -> Result<(), String> {
        document::validate_content(content)
    }

    /// Test-facing facade for the runtime-only validation policy. Production
    /// editing calls the same policy directly inside the private editor module.
    #[cfg(test)]
    pub fn validate_executable_content(content: &str) -> Result<(), String> {
        document::validate_executable_content(content)
    }

    pub fn first_active_issue(content: &str) -> Option<LuaManifestRowIssue> {
        document::first_active_issue(content)
    }

    pub fn rows_from_file(&self) -> Result<Vec<LuaManifestRow>, String> {
        let content = self.read_lua()?;
        Self::validate_content(&content)?;
        Ok(Self::rows_from_content(&content))
    }

    pub fn editor_rows_from_file(&self) -> Result<Vec<LuaManifestRow>, String> {
        let content = self.read_lua()?;
        Ok(Self::editor_rows_from_content(&content))
    }

    pub fn editor_rows_from_content(content: &str) -> Vec<LuaManifestRow> {
        document::editor_rows_from_content(content)
    }

    pub fn depot_ids_from_file(&self) -> Result<Vec<u32>, String> {
        let content = self.read_lua()?;
        let mut depot_ids: Vec<u32> = Self::rows_from_content(&content)
            .into_iter()
            .map(|row| row.app_id)
            .collect();
        depot_ids.sort_unstable();
        depot_ids.dedup();
        Ok(depot_ids)
    }

    pub fn updates_are_enabled(&self) -> Result<bool, String> {
        let content = self.read_lua()?;
        Ok(document::updates_are_enabled(&content))
    }

    pub fn set_updates_enabled(&self, enabled: bool) -> Result<usize, String> {
        let content = self.read_lua()?;
        let (next_content, changed) = editor::set_updates_enabled(&content, enabled)?;
        if let Some(next_content) = next_content {
            self.write_lua(&content, &next_content)?;
        }
        crate::desk_log_info!(
            "manifest",
            "Lua manifest {}: set_updates_enabled({}) completed -> {} pin(s) modified",
            self.lua_path.display(),
            enabled,
            changed
        );
        Ok(changed)
    }

    pub fn preview_edits(
        &self,
        edits: &[LuaManifestEdit],
    ) -> Result<(String, Vec<LuaManifestRow>), String> {
        let content = self.read_lua()?;
        editor::preview_edits(&content, edits)
    }

    /// Reads the exact Lua bytes as UTF-8 with editor-standard error context.
    pub fn read_lua(&self) -> Result<String, String> {
        fs::read_to_string(&self.lua_path)
            .map_err(|error| format!("Failed to read {}: {}", self.lua_path.display(), error))
    }

    fn write_lua(&self, expected: &str, content: &str) -> Result<(), String> {
        crate::core::state_io::write_if_unchanged(
            &self.lua_path,
            expected.as_bytes(),
            content.as_bytes(),
        )
    }
}
