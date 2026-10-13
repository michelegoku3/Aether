//! Transactional UCOnline2 deploy onto a game.
//!
//! Every deploy phase is idempotent and records its inverse action in a
//! JOURNAL (`<backup>/<app_id>/uc_online2/journal.json`): `disable()`
//! replays the journal in reverse order and restores the original tree
//! byte-for-byte (including the Neutralized files of 1.19.3 journals).
//!
//! Phases:
//!   0. pre-flight (bundle, arch, writable directories)
//!   1. original backups (Steamworks dll, ini, plugins/) → backup dir
//!   2. conflict quarantine (moved to backup/, no `*.uco-disabled`
//!      visible in the game tree — old journals with Neutralized stay
//!      supported in revert)
//!   3. UCOnline2 DLL copy (per architecture; also on refresh/upgrade)
//!   4. early overlay proxy (version.dll / XINPUT1_3.dll)
//!   5. union-crax.ini write (atomic)
//!   6. deploy of plugins required by detected backends
//!   7. state record persistence

use crate::external_tools::fs::write_atomic;
use crate::online::bundle::Uco2Bundle;
use crate::online::config::{build_ini, harvest_dlc};
use crate::online::detect::overlay_target;
use crate::online::state::{now_epoch_secs, OnlineStateStore};
use crate::online::types::{
    Conflict, DetectionReport, OnlineEnableRequest, OnlineRecord, PhotonFlavor,
};
use std::fs;
use std::path::{Path, PathBuf};

const UCO2_BACKUP_SUBDIR: &str = "uc_online2";
const JOURNAL_FILE: &str = "journal.json";
const ORIGINAL_SUBDIR: &str = "original";

/// Backup dir of a game: `<backup_root>/<app_id>/uc_online2/`.
pub fn backup_dir_for(backup_root: &Path, app_id: u32) -> PathBuf {
    backup_root
        .join(app_id.to_string())
        .join(UCO2_BACKUP_SUBDIR)
}

/// Journaled action (serialized into `journal.json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum JournalEntry {
    /// Game file moved to the backup dir. `original` = where it was,
    /// `backup` = where it is now (restore it on disable).
    BackedUp { original: PathBuf, backup: PathBuf },
    /// Neutralized file renamed to `<path>.uco-disabled` (legacy 1.19.3:
    /// new deploys use `BackedUp` and move the file to the backup dir).
    Neutralized { path: PathBuf },
    /// UCO2 file copied into the game (must be REMOVED on disable).
    Deployed { path: PathBuf },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Journal {
    pub entries: Vec<JournalEntry>,
}

impl Journal {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(JOURNAL_FILE);
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_atomic(&dir.join(JOURNAL_FILE), json.as_bytes())
    }
}

/// Run the full deploy. Idempotent: the first enable journals
/// backup/quarantine; later refreshes recopy DLL/plugin/overlay from the
/// current bundle (version upgrade) and rewrite the ini.
pub fn deploy(
    app_id: u32,
    detection: &DetectionReport,
    bundle: &Uco2Bundle,
    request: &OnlineEnableRequest,
    backup_root: &Path,
    state_path: &Path,
) -> Result<OnlineRecord, String> {
    let backup_dir = backup_dir_for(backup_root, app_id);

    // ---- Phase 0: pre-flight ----
    let steam_api_dir = detection
        .steam_api_dir
        .as_deref()
        .ok_or_else(|| "Detection found no place for steam_api(64).dll.".to_string())?;
    let ini_dir = &detection.ini_dir;
    let steam_api_path = steam_api_dir.join(detection.arch.steam_api_file_name());
    let ini_path = ini_dir.join("union-crax.ini");
    let plugins_dir = ini_dir.join("plugins");

    let mut store = OnlineStateStore::load(state_path);
    let already_enabled = store.get(app_id).is_some();

    fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(backup_dir.join(ORIGINAL_SUBDIR)).map_err(|e| e.to_string())?;
    let mut journal = Journal::load(&backup_dir);

    // UCO2 files already on disk are NOT game originals: backing them up
    // would restore them on Disable (an ini that "never goes away").
    let preexisting_uco2 = ini_path.is_file();

    if !already_enabled {
        // ---- Phase 1: original backups ----
        if steam_api_path.is_file() && !preexisting_uco2 {
            let target = unique_backup_path(&backup_dir, "steam_api");
            fs::rename(&steam_api_path, &target).map_err(|e| e.to_string())?;
            journal.entries.push(JournalEntry::BackedUp {
                original: steam_api_path.clone(),
                backup: target,
            });
        }
        if ini_path.is_file() {
            // Already UCO2: throw the ini away, don't save it as an original.
            let _ = fs::remove_file(&ini_path);
        }
        if plugins_dir.is_dir() && !preexisting_uco2 {
            let target = unique_backup_path(&backup_dir, "plugins");
            fs::rename(&plugins_dir, &target).map_err(|e| e.to_string())?;
            journal.entries.push(JournalEntry::BackedUp {
                original: plugins_dir.clone(),
                backup: target,
            });
        }

        // ---- Phase 2: conflict quarantine (outside the game tree) ----
        for conflict in &detection.conflicts {
            let path = conflict_path(conflict);
            if !path.is_file() {
                continue;
            }
            let backup_name = format!(
                "conflict_{}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("file")
            );
            let target = unique_backup_path(&backup_dir, &backup_name);
            fs::rename(&path, &target).map_err(|e| e.to_string())?;
            journal.entries.push(JournalEntry::BackedUp {
                original: path,
                backup: target,
            });
        }
        journal.save(&backup_dir)?;
    }

    // ---- Phase 3: UCOnline2 DLL copy (also on refresh: bundle bump) ----
    fs::create_dir_all(steam_api_dir).map_err(|e| e.to_string())?;
    let bundle_dll = bundle.steam_api_dll(detection.arch);
    fs::copy(&bundle_dll, &steam_api_path).map_err(|e| {
        format!(
            "Failed to copy {} -> {}: {}",
            bundle_dll.display(),
            steam_api_path.display(),
            e
        )
    })?;
    journal_deployed(&mut journal, &steam_api_path);
    journal.save(&backup_dir)?;

    // ---- Phase 4: early overlay proxy ----
    let overlay_proxy_path =
        deploy_overlay_proxy(detection, bundle, request, &backup_dir, &mut journal)?;
    journal.save(&backup_dir)?;

    // ---- Phase 5: ini (always, even for refresh) ----
    let dlc_entries = harvest_dlc(&detection.game_root, ini_dir);
    let ini_content = build_ini(detection, request, &dlc_entries);
    write_atomic(&ini_path, ini_content.as_bytes())?;
    journal_deployed(&mut journal, &ini_path);
    journal.save(&backup_dir)?;

    // ---- Phase 6: plugin ----
    let mut backends_deployed = Vec::new();
    for plugin in required_plugins(detection, request) {
        if let Some(source) = bundle.plugin_dll(plugin) {
            fs::create_dir_all(&plugins_dir).map_err(|e| e.to_string())?;
            let dest = plugins_dir.join(format!("{plugin}.dll"));
            fs::copy(&source, &dest).map_err(|e| e.to_string())?;
            backends_deployed.push(plugin.to_string());
            journal_deployed(&mut journal, &dest);
        }
    }
    if backends_deployed.is_empty() && already_enabled {
        backends_deployed = list_plugins_in(&plugins_dir);
    }
    journal.save(&backup_dir)?;

    // ---- Phase 7: record ----
    let record = OnlineRecord {
        app_id,
        enabled_at: now_epoch_secs(),
        bundle_version: bundle.version(),
        og_app_id: request.og_app_id,
        spoof_app_id: request.spoof_app_id,
        ini_path: ini_path.clone(),
        steam_api_path: steam_api_path.clone(),
        arch: detection.arch,
        backends_deployed,
        backup_dir: backup_dir.clone(),
        overlay_proxy_path,
    };
    store.upsert(record.clone(), state_path)?;

    Ok(record)
}

/// Adds `Deployed` only when the path is not already in the journal (refresh).
fn journal_deployed(journal: &mut Journal, path: &Path) {
    let already = journal.entries.iter().any(
        |entry| matches!(entry, JournalEntry::Deployed { path: existing } if existing == path),
    );
    if !already {
        journal.entries.push(JournalEntry::Deployed {
            path: path.to_path_buf(),
        });
    }
}

/// Installs the early overlay proxy. Returns the deployed path, if any.
fn deploy_overlay_proxy(
    detection: &DetectionReport,
    bundle: &Uco2Bundle,
    request: &OnlineEnableRequest,
    backup_dir: &Path,
    journal: &mut Journal,
) -> Result<Option<PathBuf>, String> {
    if !request.deploy_overlay_proxy {
        return Ok(None);
    }
    let Ok(target) = overlay_target(detection) else {
        return Ok(None);
    };
    let Some(source) = bundle.overlay_proxy_dll() else {
        return Ok(None);
    };

    if target.path.is_file() {
        let same = fs::read(&target.path)
            .ok()
            .zip(fs::read(&source).ok())
            .map(|(left, right)| left == right)
            .unwrap_or(false);
        if same {
            journal_deployed(journal, &target.path);
            return Ok(Some(target.path));
        }
        let already_backed = journal.entries.iter().any(|entry| {
            matches!(entry, JournalEntry::BackedUp { original, .. } if original == &target.path)
        });
        if !already_backed {
            let backup = unique_backup_path(backup_dir, target.file_name);
            fs::copy(&target.path, &backup).map_err(|e| e.to_string())?;
            fs::remove_file(&target.path).map_err(|e| e.to_string())?;
            journal.entries.push(JournalEntry::BackedUp {
                original: target.path.clone(),
                backup,
            });
        } else if target.path.is_file() {
            fs::remove_file(&target.path).map_err(|e| e.to_string())?;
        }
    }

    if let Some(parent) = target.path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(&source, &target.path).map_err(|e| {
        format!(
            "Failed to install overlay proxy {} -> {}: {}",
            source.display(),
            target.path.display(),
            e
        )
    })?;
    journal_deployed(journal, &target.path);
    Ok(Some(target.path))
}

/// Plugins required by the detected backends (stable order for tests/UI).
/// `overlay_proxy` is NOT an ABI plugin and does not belong in this list.
pub fn required_plugins(
    detection: &DetectionReport,
    request: &OnlineEnableRequest,
) -> Vec<&'static str> {
    let mut plugins = Vec::new();
    if request.deploy_photon && detection.backends.photon != PhotonFlavor::None {
        plugins.push("photon_universal");
    }
    if detection.backends.eos && request.deploy_eos_custom {
        plugins.push("EOS_custom");
    }
    if detection.backends.playfab {
        plugins.push("playfab_universal");
    }
    if detection.backends.coherence {
        plugins.push("coherence_universal");
    }
    plugins
}

fn conflict_path(conflict: &Conflict) -> PathBuf {
    match conflict {
        Conflict::ColdClientLoader(p)
        | Conflict::SteamFix(p)
        | Conflict::Ofme(p)
        | Conflict::NamedFixFile(p)
        | Conflict::ProxyDll(p) => p.clone(),
    }
}

/// Unique backup path inside `backup_dir/original/` (never overwrite).
fn unique_backup_path(backup_dir: &Path, base: &str) -> PathBuf {
    let original_dir = backup_dir.join(ORIGINAL_SUBDIR);
    let candidate = original_dir.join(base);
    if !candidate.exists() {
        return candidate;
    }
    for index in 1..=999 {
        let candidate = original_dir.join(format!("{base}.{index}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    original_dir.join(format!("{base}.latest"))
}

fn list_plugins_in(plugins_dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(plugins_dir) else {
        return Vec::new();
    };
    let mut plugins: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".dll") {
                Some(name[..name.len() - 4].to_string())
            } else {
                None
            }
        })
        .collect();
    plugins.sort();
    plugins
}
