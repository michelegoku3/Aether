//! Encrypted per-game UCOnline2 form preferences.
//!
//! Deployment state and user choices have different lifecycles: disabling
//! online removes deployed files/state, but must not erase values the user may
//! reuse later. Sensitive backend credentials are protected with Windows DPAPI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::external_tools::fs::write_atomic;
use crate::online::types::OnlineEnableRequest;

const PREFERENCES_FILE: &str = "uc_online2_preferences.dat";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct OnlinePreferencesStore {
    #[serde(default)]
    requests: HashMap<u32, OnlineEnableRequest>,
}

impl OnlinePreferencesStore {
    pub fn path(data_root: &Path) -> PathBuf {
        data_root.join("state").join(PREFERENCES_FILE)
    }

    pub fn load(path: &Path) -> Self {
        let Ok(encrypted) = std::fs::read(path) else {
            return Self::default();
        };
        let Ok(plain) = crate::core::secure_storage::unprotect(&encrypted) else {
            return Self::default();
        };
        serde_json::from_slice(&plain).unwrap_or_default()
    }

    pub fn get(&self, app_id: u32) -> Option<OnlineEnableRequest> {
        self.requests.get(&app_id).cloned()
    }

    pub fn upsert(
        &mut self,
        app_id: u32,
        request: OnlineEnableRequest,
        path: &Path,
    ) -> Result<(), String> {
        self.requests.insert(app_id, request);
        self.save(path)
    }

    pub fn remove(&mut self, app_id: u32, path: &Path) -> Result<(), String> {
        self.requests.remove(&app_id);
        self.save(path)
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let plain = serde_json::to_vec(self)
            .map_err(|error| format!("Could not serialize online preferences: {error}"))?;
        let encrypted = crate::core::secure_storage::protect(&plain)?;
        write_atomic(path, &encrypted)
    }
}
