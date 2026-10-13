//! Game-relative path guards: normalization, inventory-filename reservation,
//! backup-scope checks and link-aware relativization. Every crack target goes
//! through here before anything is read or written.
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::core::backup::GameBackup;

pub(crate) fn reserved_relative(relative: &str, app_id: u32) -> bool {
    let first = relative
        .split('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(['.', ' ']);
    let inventory = format!("crack_{app_id}.txt");
    first.eq_ignore_ascii_case(&inventory)
        || first.eq_ignore_ascii_case(&format!("{inventory}.tmp"))
}

pub(crate) fn normalized_relative(value: &str) -> Result<String, String> {
    let value = value.replace('\\', "/");
    if value.is_empty()
        || value.trim() != value
        || value.chars().any(|ch| matches!(ch, '\n' | '\r' | ':'))
        || !Path::new(&value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(format!("Invalid game-relative crack path: {value:?}"));
    }
    let path: PathBuf = Path::new(&value).components().collect();
    Ok(path.to_string_lossy().replace('\\', "/"))
}

pub(crate) fn check_backup_scope(game_root: &Path, backup: &GameBackup) -> Result<(), String> {
    let original_dir = backup.original_dir();
    let backup_root = original_dir.parent().ok_or("Backup has no root")?;
    if fs::canonicalize(backup_root)
        .map_err(|e| e.to_string())?
        .starts_with(fs::canonicalize(game_root).map_err(|e| e.to_string())?)
    {
        return Err(
            "Crack backup must be outside the game root; originals must not be locate targets"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn game_relative_path(target: &Path, game_root: &Path) -> Result<String, String> {
    // Resolve actual casing and reject paths through a link outside the game.
    // Saved reapply also supports parent directories removed by an earlier Remove.
    let root = fs::canonicalize(game_root).map_err(|e| e.to_string())?;
    let mut ancestor = target;
    let mut suffix = Vec::new();
    let mut resolved = loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break fs::canonicalize(ancestor).map_err(|e| e.to_string())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                suffix.push(
                    ancestor
                        .file_name()
                        .ok_or("Crack target has no filename")?
                        .to_os_string(),
                );
                ancestor = ancestor.parent().ok_or("Crack target has no parent")?;
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    let relative = resolved.strip_prefix(root).map_err(|e| e.to_string())?;
    normalized_relative(
        relative
            .to_str()
            .ok_or("Crack path is not representable in the inventory")?,
    )
}
