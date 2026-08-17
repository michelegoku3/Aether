//! Persistenza dello stato UCOnline2 per gioco.
//!
//! File: `<AetherData>/state/uc_online2.json` — mappa `app_id → OnlineRecord`.
//! Scrittura atomica (tmp + rename). Il record è un INDICE: la verità sono
//! i file sul disco (la riconciliazione vive in `OnlineEngine::status`).

use crate::online::types::OnlineRecord;
use crate::external_tools::fs::write_atomic;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Nome del file di stato UCOnline2 (usato da comandi e store — fonte unica).
pub const STATE_FILE: &str = "uc_online2.json";

/// Store dello stato UCOnline2.
#[derive(Debug, Clone, Default)]
pub struct OnlineStateStore {
    records: HashMap<u32, OnlineRecord>,
}

impl OnlineStateStore {
    /// Carica lo store dal file. File assente o corrotto → store vuoto
    /// (mai errore: lo stato è ricostruibile dai file sul disco).
    pub fn load(path: &Path) -> Self {
        let Ok(content) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Path del file di stato sotto una data root AetherDesk.
    pub fn state_path(data_root: &Path) -> PathBuf {
        data_root.join("state").join(STATE_FILE)
    }

    pub fn get(&self, app_id: u32) -> Option<&OnlineRecord> {
        self.records.get(&app_id)
    }

    /// Inserisce/aggiorna il record e salva atomicamente.
    pub fn upsert(&mut self, record: OnlineRecord, path: &Path) -> Result<(), String> {
        self.records.insert(record.app_id, record);
        self.save(path)
    }

    /// Rimuove il record (se presente) e salva atomicamente.
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
