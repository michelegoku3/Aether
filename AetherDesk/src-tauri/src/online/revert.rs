//! Rollback del deploy UCOnline2 (disattivazione).
//!
//! `disable()` replaya il JOURNAL in ordine inverso: rimuove i file UCO2
//! deployati, ripristina gli originali dalla backup dir e rinomina indietro
//! i file neutralizzati. Se il journal manca (installazione esterna), fa
//! revert euristico dai file di backup e dal record di stato.

use crate::external_tools::constants::UCO_DISABLED_SUFFIX;
use crate::online::deploy::backup_dir_for;
use crate::online::state::OnlineStateStore;
use crate::online::types::OnlineRecord;
use crate::online::deploy::{Journal, JournalEntry};
use crate::external_tools::fs::walk_files;
use std::fs;
use std::path::{Path, PathBuf};

/// Disattiva UCOnline2 per un gioco. Idempotente.
pub fn disable(app_id: u32, backup_root: &Path, state_path: &Path) -> Result<String, String> {
    let backup_dir = backup_dir_for(backup_root, app_id);
    let mut store = OnlineStateStore::load(state_path);
    let record = store.get(app_id).cloned();

    let journal = Journal::load(&backup_dir);
    if !journal.entries.is_empty() {
        revert_from_journal(&journal, &backup_dir)?;
    } else if let Some(record) = &record {
        revert_heuristic(record)?;
    }

    store.remove(app_id, state_path)?;
    Ok("UCOnline2 disabled: files restored and state cleared.".to_string())
}

/// Revert guidato dal journal (ordine inverso).
fn revert_from_journal(journal: &Journal, backup_dir: &Path) -> Result<(), String> {
    let mut plugin_dirs: Vec<PathBuf> = Vec::new();
    for entry in journal.entries.iter().rev() {
        match entry {
            JournalEntry::Deployed { path } => {
                if path.is_file() {
                    fs::remove_file(path).map_err(|e| e.to_string())?;
                }
                // Ricorda la cartella plugins\ di provenienza (se vuota la
                // rimuoviamo dopo: era nostra, il gioco non l'aveva).
                if let Some(parent) = path.parent() {
                    let is_plugins = parent
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.eq_ignore_ascii_case("plugins"))
                        .unwrap_or(false);
                    if is_plugins && !plugin_dirs.iter().any(|p| p == parent) {
                        plugin_dirs.push(parent.to_path_buf());
                    }
                }
            }
            JournalEntry::Neutralized { path } => {
                // path = nome originale; il file attuale è path + suffisso.
                let disabled = path.with_file_name(format!(
                    "{}{}",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
                    UCO_DISABLED_SUFFIX
                ));
                if disabled.is_file() && !path.exists() {
                    fs::rename(&disabled, path).map_err(|e| e.to_string())?;
                }
            }
            JournalEntry::BackedUp { original, backup } => {
                // Files AND directories (the original plugins/ folder is a dir).
                if backup.exists() && !original.exists() {
                    if let Some(parent) = original.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    fs::rename(backup, original).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // Rimuove le cartelle plugins\ ormai vuote (best-effort).
    for dir in plugin_dirs {
        let empty = dir
            .read_dir()
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if empty {
            let _ = fs::remove_dir(dir);
        }
    }

    // Rimuove journal e, se non restano file, l'intera struttura di backup.
    let _ = fs::remove_file(backup_dir.join("journal.json"));
    let leftovers: Vec<PathBuf> = walk_files(backup_dir);
    if leftovers.is_empty() {
        let _ = fs::remove_dir_all(backup_dir);
    }

    Ok(())
}

/// Revert euristico quando il journal manca: ripristina dagli originali
/// nella backup dir (se esistono) e rimuove i file deployati noti dal
/// record. Best-effort: non tocca file che non riconosce.
fn revert_heuristic(record: &OnlineRecord) -> Result<(), String> {
    // 1. Rimuove ini e dll deployate (solo se esistono e il record le indica).
    if record.ini_path.is_file() {
        fs::remove_file(&record.ini_path).map_err(|e| e.to_string())?;
    }
    if record.steam_api_path.is_file() {
        fs::remove_file(&record.steam_api_path).map_err(|e| e.to_string())?;
    }
    if let Some(overlay) = &record.overlay_proxy_path {
        if overlay.is_file() {
            fs::remove_file(overlay).map_err(|e| e.to_string())?;
        }
    }
    // 2. Rimuove la cartella plugins deployata (tutta: è nostra).
    if let Some(plugins_dir) = record.ini_path.parent().map(|d| d.join("plugins")) {
        if plugins_dir.is_dir() {
            fs::remove_dir_all(&plugins_dir).map_err(|e| e.to_string())?;
        }
    }
    // 3. Ripristina gli originali dalla backup dir se presenti.
    let original_dir = record.backup_dir.join("original");
    if original_dir.is_dir() {
        for file in walk_files(&original_dir) {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let target = match name {
                "steam_api64.dll" | "steam_api.dll" => {
                    record.steam_api_path.parent().map(|p| p.join(name))
                }
                "union-crax.ini" => record.ini_path.parent().map(|p| p.join(name)),
                _ => None,
            };
            if let Some(target) = target {
                if file.is_file() && !target.exists() {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    fs::rename(&file, target).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    let _ = fs::remove_dir_all(&record.backup_dir);
    Ok(())
}
