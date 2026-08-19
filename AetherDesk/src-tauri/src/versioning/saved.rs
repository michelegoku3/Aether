use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::versioning::cache::now_unix;
use crate::versioning::model::SavedBuild;

const SAVED_FILE_NAME: &str = "saved_builds.json";
const SAVED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedBuildsFile {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    builds: Vec<SavedBuild>,
}

/// Persistent bookmarks of builds the user wants quick access to
/// (`AetherData/saved_builds.json`). Deduplicated by (app_id, build_id);
/// saving an existing pair refreshes its `saved_at`.
pub struct SavedBuildsStore {
    path: PathBuf,
}

impl SavedBuildsStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            path: data_dir.join(SAVED_FILE_NAME),
        }
    }

    /// Saved builds of one app, newest save first.
    pub fn list(&self, app_id: u32) -> Vec<SavedBuild> {
        let mut builds: Vec<SavedBuild> = self
            .load()
            .builds
            .into_iter()
            .filter(|b| b.app_id == app_id)
            .collect();
        builds.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
        builds
    }

    pub fn add(&self, build: SavedBuild) -> Result<SavedBuild, String> {
        let mut file = self.load();
        file.schema_version = SAVED_SCHEMA_VERSION;
        let saved_at = now_unix();
        if let Some(existing) = file
            .builds
            .iter_mut()
            .find(|b| b.app_id == build.app_id && b.build_id == build.build_id)
        {
            existing.date = build.date;
            existing.title = build.title;
            existing.saved_at = saved_at;
            let saved = existing.clone();
            self.save(&file)?;
            return Ok(saved);
        }
        let saved = SavedBuild { saved_at, ..build };
        file.builds.push(saved.clone());
        self.save(&file)?;
        Ok(saved)
    }

    pub fn remove(&self, app_id: u32, build_id: u64) -> Result<(), String> {
        let mut file = self.load();
        let before = file.builds.len();
        file.builds
            .retain(|b| !(b.app_id == app_id && b.build_id == build_id));
        if file.builds.len() == before {
            return Ok(()); // nothing to remove
        }
        self.save(&file)
    }

    fn load(&self) -> SavedBuildsFile {
        let Ok(content) = fs::read_to_string(&self.path) else {
            return SavedBuildsFile::default();
        };
        match serde_json::from_str::<SavedBuildsFile>(&content) {
            Ok(file) if file.schema_version == SAVED_SCHEMA_VERSION => file,
            _ => SavedBuildsFile::default(),
        }
    }

    fn save(&self, file: &SavedBuildsFile) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create data dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(file)
            .map_err(|e| format!("Failed to serialize saved builds: {e}"))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| format!("Failed to write saved builds: {e}"))?;
        fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Failed to commit saved builds: {e}"))
    }
}
