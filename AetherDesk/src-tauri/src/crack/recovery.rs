//! Crack recovery inventory: which files were applied, what the originals
//! were, and what is pending. Guards every mutation (`known_content`,
//! `verify_original`) and persists the journal (`load_recovery`,
//! `save_recovery`, including legacy-inventory repair).
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::backup::GameBackup;
use crate::external_tools::fs::write_atomic;

use super::hashing::optional_hash;
use super::paths::{normalized_relative, reserved_relative};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoveryFile {
    // None means the target was absent before the first tracked application.
    pub(crate) had_original: bool,
    pub(crate) original_sha256: Option<String>,
    pub(crate) applied_sha256: Option<String>,
    pub(crate) pending_sha256: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RecoveryInventory {
    pub(crate) version: u32,
    pub(crate) files: BTreeMap<String, RecoveryFile>,
}

pub(crate) fn known_content(record: &RecoveryFile, hash: &str) -> bool {
    [
        record.original_sha256.as_deref(),
        record.applied_sha256.as_deref(),
        record.pending_sha256.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|known| known == hash)
}

pub(crate) fn verify_original(path: &Path, record: &RecoveryFile) -> Result<(), String> {
    if optional_hash(path)? != record.original_sha256 {
        return Err(format!(
            "Original backup missing or changed at {}; operation refused",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn load_recovery(backup: &GameBackup, app_id: u32) -> Result<RecoveryInventory, String> {
    let state_path = backup.crack_recovery_path(app_id);
    let state = match fs::read_to_string(&state_path) {
        Ok(content) => {
            let state: RecoveryInventory = serde_json::from_str(&content).map_err(|e| {
                format!("Invalid crack recovery state {}: {e}", state_path.display())
            })?;
            if state.version != 1 {
                return Err("Unsupported crack recovery state version".into());
            }
            state
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let inventory_path = backup.crack_inventory_path(app_id);
            if inventory_path.try_exists().map_err(|e| e.to_string())? && !inventory_path.is_file()
            {
                return Err(format!(
                    "Invalid crack inventory at {}",
                    inventory_path.display()
                ));
            }
            // Repair earlier multi-source inventories from the saved tree as well.
            let mut paths = backup.read_crack_inventory(app_id)?;
            paths.extend(backup.list_saved_crack_files()?);
            let mut files = BTreeMap::new();
            for path in paths {
                let path = normalized_relative(&path)?;
                if reserved_relative(&path, app_id) {
                    return Err("Legacy crack path collides with inventory metadata".into());
                }
                let original_sha256 = optional_hash(&backup.original_dir().join(&path))?;
                let applied_sha256 = optional_hash(&backup.crack_dir().join(&path))?;
                if applied_sha256.is_none() {
                    return Err(format!(
                        "Saved crack bytes missing for legacy inventory path {path}"
                    ));
                }
                files.insert(
                    path,
                    RecoveryFile {
                        had_original: original_sha256.is_some(),
                        original_sha256,
                        applied_sha256,
                        pending_sha256: None,
                    },
                );
            }
            RecoveryInventory { version: 1, files }
        }
        Err(error) => {
            return Err(format!(
                "Read crack recovery state {}: {error}",
                state_path.display()
            ))
        }
    };
    for (path, record) in &state.files {
        if record.had_original != record.original_sha256.is_some() {
            return Err("Incomplete original ownership in crack recovery state".into());
        }
        if reserved_relative(path, app_id) || normalized_relative(path)? != *path {
            return Err("Non-normalized crack recovery path".into());
        }
        for hash in [
            record.original_sha256.as_ref(),
            record.applied_sha256.as_ref(),
            record.pending_sha256.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("Invalid hash in crack recovery state".into());
            }
        }
    }
    Ok(state)
}

pub(crate) fn save_recovery(
    backup: &GameBackup,
    app_id: u32,
    state: &RecoveryInventory,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    write_atomic(&backup.crack_recovery_path(app_id), &bytes)?;
    let paths: Vec<_> = state
        .files
        .iter()
        .filter(|(_, record)| record.applied_sha256.is_some() || record.pending_sha256.is_some())
        .map(|(path, _)| path.as_str())
        .collect();
    if paths.is_empty() {
        backup.clear_crack_inventory(app_id)
    } else {
        write_atomic(
            &backup.crack_inventory_path(app_id),
            format!("{}\n", paths.join("\n")).as_bytes(),
        )
    }
}
