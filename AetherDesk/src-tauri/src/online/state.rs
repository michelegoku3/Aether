//! Per-game UCOnline2 state persistence.
//!
//! File: `<AetherData>/state/uc_online2.json` — `app_id → OnlineRecord` map.
//! Atomic writes (tmp + rename). The record is an INDEX: the truth is the
//! files on disk (reconciliation lives in `OnlineEngine::status`).

use crate::external_tools::fs::write_atomic;
use crate::online::types::OnlineRecord;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// UCOnline2 state file name (used by commands and store — single source).
pub const STATE_FILE: &str = "uc_online2.json";

/// UCOnline2 state store.
#[derive(Debug, Clone, Default)]
pub struct OnlineStateStore {
    records: HashMap<u32, OnlineRecord>,
}

impl OnlineStateStore {
    /// Loads the store from file. Missing or corrupt file → empty store
    /// (never an error: the state is rebuildable from the files on disk).
    pub fn load(path: &Path) -> Self {
        let Ok(content) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// State file path under a given AetherDesk data root.
    pub fn state_path(data_root: &Path) -> PathBuf {
        data_root.join("state").join(STATE_FILE)
    }

    pub fn get(&self, app_id: u32) -> Option<&OnlineRecord> {
        self.records.get(&app_id)
    }

    /// Inserts/updates the record and saves atomically.
    pub fn upsert(&mut self, record: OnlineRecord, path: &Path) -> Result<(), String> {
        self.records.insert(record.app_id, record);
        self.save(path)
    }

    /// Removes the record (if present) and saves atomically.
    pub fn remove(&mut self, app_id: u32, path: &Path) -> Result<(), String> {
        self.records.remove(&app_id);
        self.save(path)
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_atomic(path, json.as_bytes())
    }
}

impl serde::Serialize for OnlineStateStore {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.records.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for OnlineStateStore {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let records = HashMap::<u32, OnlineRecord>::deserialize(deserializer)?;
        Ok(Self { records })
    }
}

/// Timestamp epoch (secondi) per `OnlineRecord::enabled_at`.
pub fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
