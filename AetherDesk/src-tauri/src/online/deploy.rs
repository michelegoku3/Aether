//! Deploy transazionale di UCOnline2 su un gioco.
//!
//! Ogni fase del deploy è idempotente e registra la propria azione inversa
//! in un JOURNAL (`<backup>/<app_id>/uc_online2/journal.json`): `disable()`
//! replaya il journal in ordine inverso e ripristina l'albero originale
//! byte-per-byte (inclusi i file Neutralized dei journal 1.19.3).
//!
//! Fasi:
//!   0. pre-flight (bundle, arch, directory scrivibili)
//!   1. backup originali (dll Steamworks, ini, plugins/) → backup dir
//!   2. quarantena conflitti (spostati in backup/, niente `*.uco-disabled`
//!      visibili nel game tree — i journal vecchi con Neutralized restano
//!      supportati in revert)
//!   3. copia DLL UCOnline2 (per architettura; anche in refresh/upgrade)
//!   4. early overlay proxy (version.dll / XINPUT1_3.dll)
//!   5. scrittura union-crax.ini (atomica)
//!   6. deploy plugin richiesti dai backend rilevati
//!   7. persistenza del record di stato

use crate::online::bundle::Uco2Bundle;
use crate::online::config::{build_ini, harvest_dlc};
use crate::online::detect::overlay_target;
use crate::online::state::{now_epoch_secs, OnlineStateStore};
use crate::online::types::{
    Conflict, DetectionReport, OnlineEnableRequest, OnlineRecord, PhotonFlavor,
};
use crate::external_tools::fs::write_atomic;
use std::fs;
use std::path::{Path, PathBuf};

const UCO2_BACKUP_SUBDIR: &str = "uc_online2";
const JOURNAL_FILE: &str = "journal.json";
const ORIGINAL_SUBDIR: &str = "original";

/// Backup dir di un gioco: `<backup_root>/<app_id>/uc_online2/`.
pub fn backup_dir_for(backup_root: &Path, app_id: u32) -> PathBuf {
    backup_root
        .join(app_id.to_string())
        .join(UCO2_BACKUP_SUBDIR)
}

/// Azione journalizzata (serializzata in `journal.json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum JournalEntry {
    /// File di gioco spostato nella backup dir. `original` = dove stava,
    /// `backup` = dove è ora (va ripristinato in disable).
    BackedUp { original: PathBuf, backup: PathBuf },
    /// File neutralizzato rinominato in `<path>.uco-disabled` (legacy 1.19.3:
    /// i deploy nuovi usano `BackedUp` e spostano il file nel backup dir).
    Neutralized { path: PathBuf },
    /// File UCO2 copiato nel gioco (va RIMOSSO in disable).
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

/// Esegue il deploy completo. Idempotente: al primo enable journalizza
/// backup/quarantena; ai refresh successivi ricopia DLL/plugin/overlay dal
/// bundle corrente (upgrade di versione) e riscrive l'ini.
pub fn deploy(
    app_id: u32,
    detection: &DetectionReport,
    bundle: &Uco2Bundle,
    request: &OnlineEnableRequest,
    backup_root: &Path,
    state_path: &Path,
) -> Result<OnlineRecord, String> {
    let backup_dir = backup_dir_for(backup_root, app_id);

    // ---- Fase 0: pre-flight ----
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

    // File UCO2 già sul disco NON sono originali del gioco: backuparli
    // farebbe ripristinarli al Disable (ini che "non se ne va").
    let preexisting_uco2 = ini_path.is_file();

    if !already_enabled {
        // ---- Fase 1: backup originali ----
        if steam_api_path.is_file() && !preexisting_uco2 {
            let target = unique_backup_path(&backup_dir, "steam_api");
            fs::rename(&steam_api_path, &target).map_err(|e| e.to_string())?;
            journal.entries.push(JournalEntry::BackedUp {
                original: steam_api_path.clone(),
                backup: target,
            });
        }
        if ini_path.is_file() {
            // Già UCO2: butta l'ini, non lo salvare come originale.
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

        // ---- Fase 2: quarantena conflitti (fuori dal game tree) ----
        for conflict in &detection.conflicts {
            let path = conflict_path(conflict);
            if !path.is_file() {
                continue;
            }
            let backup_name = format!(
                "conflict_{}",
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
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

    // ---- Fase 3: copia DLL UCOnline2 (anche in refresh: bump bundle) ----
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

    // ---- Fase 4: early overlay proxy ----
    let overlay_proxy_path =
        deploy_overlay_proxy(detection, bundle, request, &backup_dir, &mut journal)?;
    journal.save(&backup_dir)?;

    // ---- Fase 5: ini (sempre, anche per refresh) ----
    let dlc_entries = harvest_dlc(&detection.game_root, ini_dir);
    let ini_content = build_ini(detection, request, &dlc_entries);
    write_atomic(&ini_path, ini_content.as_bytes())?;
    journal_deployed(&mut journal, &ini_path);
    journal.save(&backup_dir)?;

    // ---- Fase 6: plugin ----
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

    // ---- Fase 7: record ----
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

/// Aggiunge `Deployed` solo se il path non è già nel journal (refresh).
fn journal_deployed(journal: &mut Journal, path: &Path) {
    let already = journal.entries.iter().any(|entry| {
        matches!(entry, JournalEntry::Deployed { path: existing } if existing == path)
    });
    if !already {
        journal
            .entries
            .push(JournalEntry::Deployed { path: path.to_path_buf() });
    }
}

/// Installa l'early overlay proxy. Restituisce il path deployato, se c'è.
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

/// Plugin richiesti dai backend rilevati (ordine stabile per test/UI).
/// `overlay_proxy` NON è un plugin ABI e non va in questa lista.
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
        | Conflict::OnlineFix(p)
        | Conflict::NamedFixFile(p)
        | Conflict::ProxyDll(p) => p.clone(),
    }
}

/// Path di backup univoco dentro `backup_dir/original/` (mai sovrascrivere).
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
