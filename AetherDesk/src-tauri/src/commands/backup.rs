use crate::core::paths::LocalAppPaths;
use crate::steam::app_names::SteamAppNameResolver;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Serialize)]
pub struct BackupGameSummary {
    pub app_id: u32,
    pub game_name: Option<String>,
    pub has_lua_original: bool,
    pub history_count: usize,
    pub manifest_count: usize,
    pub has_crack: bool,
    pub has_original_files: bool,
    pub has_achievements: bool,
    pub total_size_bytes: u64,
    pub backup_path: String,
}

fn dir_has_files(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() || (path.is_dir() && dir_has_files(&path)) {
            return true;
        }
    }
    false
}

fn dir_size_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total = total.saturating_add(dir_size_bytes(&path));
        } else if let Ok(meta) = entry.metadata() {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

fn collect_backup_app_ids(root: &Path) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
        })
        .collect()
}

pub(crate) fn list_backup_games_in(
    root: &Path,
    cached_names: &HashMap<u32, String>,
) -> Vec<BackupGameSummary> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(app_id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let lua_dir = path.join("lua");
        let has_lua_original = lua_dir.join(format!("{app_id}.lua")).is_file();
        let history_dir = lua_dir.join("history");
        let history_count = fs::read_dir(&history_dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("lua"))
                    .count()
            })
            .unwrap_or(0);
        let manifest_count = fs::read_dir(&lua_dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("manifest"))
                    .count()
            })
            .unwrap_or(0);
        let has_crack = dir_has_files(&path.join("crack"));
        let has_original_files = dir_has_files(&path.join("original"));
        let has_achievements = dir_has_files(&path.join("achievements"));
        let total_size_bytes = dir_size_bytes(&path);
        out.push(BackupGameSummary {
            app_id,
            game_name: cached_names.get(&app_id).cloned(),
            has_lua_original,
            history_count,
            manifest_count,
            has_crack,
            has_original_files,
            has_achievements,
            total_size_bytes,
            backup_path: path.display().to_string(),
        });
    }
    out.sort_by_key(|s| s.app_id);
    out
}

pub(crate) fn prune_lua_history_in_dir(
    history_dir: &Path,
    keep_latest: usize,
) -> Result<usize, String> {
    if !history_dir.is_dir() {
        return Ok(0);
    }
    let mut files: Vec<(u128, String, std::path::PathBuf)> = Vec::new();
    let entries =
        fs::read_dir(history_dir).map_err(|e| format!("Failed to read Lua history folder: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|x| x.to_str()) != Some("lua") {
            continue;
        }
        let modified_ns = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = entry.file_name().to_string_lossy().into_owned();
        files.push((modified_ns, name, path));
    }
    // Order newest-first by filename timestamp (with mtime tie-breaker), keeping top `keep_latest`.
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    let mut removed = 0usize;
    for (_, _, path) in files.into_iter().skip(keep_latest) {
        fs::remove_file(&path)
            .map_err(|e| format!("Failed to remove history archive {}: {e}", path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

pub(crate) fn delete_backup_in_root(root: &Path, app_id: u32) -> Result<String, String> {
    if app_id == 0 {
        return Err("A valid Steam App ID is required".to_string());
    }
    let target = root.join(app_id.to_string());
    if !target.is_dir() {
        return Err(format!("No backup folder found for App ID {app_id}."));
    }
    fs::remove_dir_all(&target)
        .map_err(|e| format!("Failed to delete backup for App ID {app_id}: {e}"))?;
    Ok(format!("Deleted backup for App ID {app_id}."))
}

#[tauri::command]
pub fn list_backup_games() -> Result<Vec<BackupGameSummary>, String> {
    let root = LocalAppPaths::backup_root();
    let app_ids = collect_backup_app_ids(&root);
    let cache_dir = LocalAppPaths::data_root().join("cache");
    let cached_names = SteamAppNameResolver::new(&cache_dir).cached_names(app_ids);
    Ok(list_backup_games_in(&root, &cached_names))
}

#[tauri::command]
pub fn prune_backup_lua_history(app_id: u32, keep_latest: usize) -> Result<usize, String> {
    if app_id == 0 {
        return Err("A valid Steam App ID is required".to_string());
    }
    let history_dir = LocalAppPaths::backup_root()
        .join(app_id.to_string())
        .join("lua")
        .join("history");
    let removed = prune_lua_history_in_dir(&history_dir, keep_latest)?;
    crate::desk_log_info!(
        "backup",
        "Pruned {} old Lua history file(s) for AppID {} (kept {})",
        removed,
        app_id,
        keep_latest
    );
    Ok(removed)
}

#[tauri::command]
pub fn delete_game_backup(app_id: u32) -> Result<String, String> {
    let root = LocalAppPaths::backup_root();
    let msg = delete_backup_in_root(&root, app_id)?;
    crate::desk_log_info!("backup", "{}", msg);
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::{delete_backup_in_root, list_backup_games_in, prune_lua_history_in_dir};
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn lists_game_names_sizes_and_achievements() {
        let dir = tempfile::tempdir().unwrap();
        let app_dir = dir.path().join("730");
        fs::create_dir_all(app_dir.join("lua/history")).unwrap();
        fs::create_dir_all(app_dir.join("achievements")).unwrap();
        fs::write(app_dir.join("lua/730.lua"), b"addappid(730)").unwrap();
        fs::write(app_dir.join("lua/731_1.manifest"), b"12345").unwrap();
        fs::write(app_dir.join("lua/history/730-100.lua"), b"old").unwrap();
        fs::write(app_dir.join("achievements/UserGameStats_1_730.json"), b"{}").unwrap();

        let mut names = HashMap::new();
        names.insert(730, "Counter-Strike 2".to_string());

        let list = list_backup_games_in(dir.path(), &names);
        assert_eq!(list.len(), 1);
        let item = &list[0];
        assert_eq!(item.app_id, 730);
        assert_eq!(item.game_name.as_deref(), Some("Counter-Strike 2"));
        assert!(item.has_lua_original);
        assert_eq!(item.history_count, 1);
        assert_eq!(item.manifest_count, 1);
        assert!(item.has_achievements);
        assert_eq!(item.total_size_bytes, 13 + 5 + 3 + 2);
    }

    #[test]
    fn prunes_oldest_lua_history_and_keeps_non_lua_files() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("history");
        fs::create_dir_all(&history).unwrap();
        for name in ["730-100.lua", "730-200.lua", "730-300.lua", "730-400.lua"] {
            fs::write(history.join(name), b"lua").unwrap();
        }
        fs::write(history.join("notes.txt"), b"keep").unwrap();

        let removed = prune_lua_history_in_dir(&history, 2).unwrap();
        assert_eq!(removed, 2);
        assert!(history.join("730-400.lua").is_file());
        assert!(history.join("730-300.lua").is_file());
        assert!(!history.join("730-200.lua").exists());
        assert!(!history.join("730-100.lua").exists());
        assert!(history.join("notes.txt").is_file());
    }

    #[test]
    fn deletes_only_target_app_backup_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("730/lua")).unwrap();
        fs::create_dir_all(dir.path().join("570/lua")).unwrap();

        let msg = delete_backup_in_root(dir.path(), 730).unwrap();
        assert!(msg.contains("730"));
        assert!(!dir.path().join("730").exists());
        assert!(dir.path().join("570").is_dir());
        assert!(delete_backup_in_root(dir.path(), 0).is_err());
        assert!(delete_backup_in_root(dir.path(), 999).is_err());
    }
}
