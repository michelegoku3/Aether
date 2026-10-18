// Applying crack files with immutable originals and recoverable per-file commits.
// The legacy crack_<app_id>.txt inventory remains a line-per-path list. A private
// sidecar records original absence and applied/pending hashes across all sources.

use crate::core::backup::GameBackup;
use crate::crack::locate;
use std::fs;
use std::path::Path;

use super::hashing::{hash_file, optional_hash};
use super::io::{atomic_copy, collect_files, remove_empty_parents};
use super::paths::{
    check_backup_scope, game_relative_path, normalized_relative, reserved_relative,
};
use super::recovery::{
    known_content, load_recovery, save_recovery, verify_original, RecoveryFile, RecoveryInventory,
};

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub applied: usize,
    pub replaced: usize,
    pub files: Vec<String>,
}

pub fn apply_staged_files(
    app_id: u32,
    staging: &Path,
    game_root: &Path,
    backup: &GameBackup,
) -> Result<ApplyReport, String> {
    let staged = collect_files(staging)?;
    if staged.is_empty() {
        return Err("No crack files found in the selected archive.".into());
    }
    check_backup_scope(game_root, backup)?;
    if fs::canonicalize(staging)
        .map_err(|e| e.to_string())?
        .starts_with(fs::canonicalize(game_root).map_err(|e| e.to_string())?)
    {
        return Err("Crack staging must be outside the game root".into());
    }
    let mut inventory = load_recovery(backup, app_id)?;
    let mut report = ApplyReport::default();
    let is_flat = staged.iter().all(|path| {
        path.strip_prefix(staging)
            .is_ok_and(|rel| rel.components().count() == 1)
    });
    for source in &staged {
        let relative = source.strip_prefix(staging).map_err(|e| e.to_string())?;
        for target in locate::resolve_file_targets(relative, game_root, is_flat) {
            let relative = recorded_casing(
                game_relative_path(&target, game_root)?,
                game_root,
                &inventory,
            );
            let target = game_root.join(&relative);
            apply_one(
                app_id,
                source,
                &target,
                &relative,
                backup,
                &mut inventory,
                &mut report,
            )?;
        }
    }
    Ok(report)
}

pub fn reapply_saved_crack_files(
    app_id: u32,
    game_root: &Path,
    backup: &GameBackup,
) -> Result<ApplyReport, String> {
    check_backup_scope(game_root, backup)?;
    let saved = backup.list_saved_crack_files()?;
    if saved.is_empty() {
        return Err("No saved crack files found in the backup folder.".into());
    }
    let mut inventory = load_recovery(backup, app_id)?;
    let mut report = ApplyReport::default();
    for relative in saved {
        let relative = normalized_relative(&relative)?;
        let source = backup.crack_dir().join(&relative);
        let target = game_root.join(&relative);
        let relative = recorded_casing(
            game_relative_path(&target, game_root)?,
            game_root,
            &inventory,
        );
        let target = game_root.join(&relative);
        apply_one(
            app_id,
            &source,
            &target,
            &relative,
            backup,
            &mut inventory,
            &mut report,
        )?;
    }
    Ok(report)
}

fn apply_one(
    app_id: u32,
    source: &Path,
    target: &Path,
    relative: &str,
    backup: &GameBackup,
    inventory: &mut RecoveryInventory,
    report: &mut ApplyReport,
) -> Result<(), String> {
    let relative = normalized_relative(relative)?;
    // This legacy filename lives in original/, so it cannot also hold a game original.
    if reserved_relative(&relative, app_id) {
        return Err("Crack target conflicts with the recovery inventory filename".into());
    }
    let incoming = hash_file(source)?;
    let current = optional_hash(target)?;
    let original = backup.original_dir().join(&relative);
    if !inventory.files.contains_key(&relative) {
        let original_sha256 = match optional_hash(&original)? {
            Some(hash) => Some(hash),
            None => match &current {
                Some(hash) => {
                    atomic_copy(target, &original, hash, false)?;
                    Some(hash.clone())
                }
                None => None,
            },
        };
        inventory.files.insert(
            relative.clone(),
            RecoveryFile {
                had_original: original_sha256.is_some(),
                original_sha256,
                applied_sha256: None,
                pending_sha256: None,
            },
        );
    }
    let record = inventory.files.get_mut(&relative).expect("record inserted");
    verify_original(&original, record)?;
    if let Some(hash) = &current {
        if !known_content(record, hash) {
            return Err(format!(
                "Crack target changed outside the recorded operation: {}; no overwrite",
                target.display()
            ));
        }
        report.replaced += 1;
    }
    // Write ahead: a failed copy must leave enough ownership data for retry/removal.
    record.pending_sha256 = Some(incoming.clone());
    save_recovery(backup, app_id, inventory)?;

    let saved = backup.crack_dir().join(&relative);
    if source != saved.as_path() {
        atomic_copy(source, &saved, &incoming, true)?;
    }
    atomic_copy(source, target, &incoming, true)?;
    let record = inventory
        .files
        .get_mut(&relative)
        .expect("record remains present");
    record.applied_sha256 = Some(incoming);
    record.pending_sha256 = None;
    save_recovery(backup, app_id, inventory)?;
    report.applied += 1;
    report.files.push(relative);
    crate::desk_log_info!(
        "crack",
        "Applied crack file: {} -> {}",
        source.display(),
        target.display()
    );
    Ok(())
}

/// Remove only verified applied bytes; pristine originals remain in the backup.
pub fn remove_applied_crack_files(
    app_id: u32,
    game_root: &Path,
    backup: &GameBackup,
) -> Result<usize, String> {
    check_backup_scope(game_root, backup)?;
    let mut inventory = load_recovery(backup, app_id)?;
    let active: Vec<_> = inventory
        .files
        .iter()
        .filter(|(_, record)| record.applied_sha256.is_some() || record.pending_sha256.is_some())
        .map(|(relative, _)| relative.clone())
        .collect();
    let mut removed = 0;
    for relative in active {
        let target = game_root.join(&relative);
        let _ = game_relative_path(&target, game_root)?;
        let record = inventory.files.get(&relative).expect("active record");
        verify_original(&backup.original_dir().join(&relative), record)?;
        if let Some(current) = optional_hash(&target)? {
            if record.original_sha256.as_ref() == Some(&current) {
                // A pending commit may have failed before touching the live original.
            } else if record.applied_sha256.as_ref() == Some(&current)
                || record.pending_sha256.as_ref() == Some(&current)
            {
                fs::remove_file(&target)
                    .map_err(|e| format!("Remove {}: {e}", target.display()))?;
                removed += 1;
                remove_empty_parents(&target, game_root);
            } else {
                return Err(format!(
                    "Crack target {} has unrecognized bytes; recovery retained",
                    target.display()
                ));
            }
        }
        let record = inventory.files.get_mut(&relative).expect("active record");
        record.applied_sha256 = None;
        record.pending_sha256 = None;
        save_recovery(backup, app_id, &inventory)?;
    }
    // Also repairs/clears a legacy inventory after an interrupted final save.
    save_recovery(backup, app_id, &inventory)?;
    Ok(removed)
}

fn recorded_casing(relative: String, game_root: &Path, inventory: &RecoveryInventory) -> String {
    if cfg!(windows) {
        if let Ok(identity) = fs::canonicalize(game_root.join(&relative)) {
            for recorded in inventory.files.keys() {
                if recorded.to_lowercase() == relative.to_lowercase()
                    && fs::canonicalize(game_root.join(recorded)).ok().as_ref() == Some(&identity)
                {
                    return recorded.clone();
                }
            }
        }
    }
    relative
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    const APP_ID: u32 = 424242;

    fn setup() -> (tempfile::TempDir, GameBackup, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("backup");
        for directory in ["lua", "original", "crack"] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        let game = temp.path().join("game");
        let staging = temp.path().join("stage");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(temp.path().join("protected-sentinel.txt"), b"protected").unwrap();
        (temp, GameBackup::for_tests(root), game, staging)
    }

    #[test]
    fn repeated_import_and_saved_reapply_keep_the_first_original() {
        let (_temp, backup, game, staging) = setup();
        fs::write(game.join("api.dll"), b"pristine").unwrap();
        fs::write(staging.join("api.dll"), b"patch one").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        fs::write(staging.join("api.dll"), b"patch two").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        reapply_saved_crack_files(APP_ID, &game, &backup).unwrap();
        assert_eq!(
            fs::read(backup.original_dir().join("api.dll")).unwrap(),
            b"pristine"
        );
        assert_eq!(fs::read(game.join("api.dll")).unwrap(), b"patch two");
        assert_eq!(
            backup.read_crack_inventory(APP_ID).unwrap(),
            vec!["api.dll"]
        );
    }

    #[test]
    fn disjoint_imports_keep_complete_inventory_and_remove_all_applied_files() {
        let (_temp, backup, game, staging) = setup();
        fs::write(game.join("a.dll"), b"original a").unwrap();
        fs::write(game.join("b.dll"), b"original b").unwrap();
        fs::write(staging.join("a.dll"), b"patch a").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        fs::remove_file(staging.join("a.dll")).unwrap();
        fs::write(staging.join("b.dll"), b"patch b").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        assert_eq!(
            backup.read_crack_inventory(APP_ID).unwrap(),
            vec!["a.dll", "b.dll"]
        );
        assert_eq!(
            remove_applied_crack_files(APP_ID, &game, &backup).unwrap(),
            2
        );
        assert!(!game.join("a.dll").exists());
        assert!(!game.join("b.dll").exists());
        assert_eq!(
            fs::read(backup.original_dir().join("a.dll")).unwrap(),
            b"original a"
        );
        assert_eq!(
            fs::read(backup.original_dir().join("b.dll")).unwrap(),
            b"original b"
        );
        assert_eq!(
            remove_applied_crack_files(APP_ID, &game, &backup).unwrap(),
            0
        );
    }

    #[test]
    fn originally_absent_file_never_becomes_an_original_on_reapply() {
        let (_temp, backup, game, staging) = setup();
        fs::write(staging.join("added.dll"), b"patch one").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        fs::write(staging.join("added.dll"), b"patch two").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        reapply_saved_crack_files(APP_ID, &game, &backup).unwrap();
        assert!(!backup.original_dir().join("added.dll").exists());
        assert!(load_recovery(&backup, APP_ID).unwrap().files["added.dll"]
            .original_sha256
            .is_none());
        assert_eq!(
            remove_applied_crack_files(APP_ID, &game, &backup).unwrap(),
            1
        );
        reapply_saved_crack_files(APP_ID, &game, &backup).unwrap();
        assert!(!backup.original_dir().join("added.dll").exists());
        assert_eq!(fs::read(game.join("added.dll")).unwrap(), b"patch two");
    }

    #[test]
    fn unknown_live_edit_is_not_overwritten_or_removed() {
        let (_temp, backup, game, staging) = setup();
        fs::write(game.join("api.dll"), b"pristine").unwrap();
        fs::write(staging.join("api.dll"), b"patch").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        fs::write(game.join("api.dll"), b"user edit sentinel").unwrap();
        assert!(apply_staged_files(APP_ID, &staging, &game, &backup).is_err());
        assert!(remove_applied_crack_files(APP_ID, &game, &backup).is_err());
        assert_eq!(
            fs::read(game.join("api.dll")).unwrap(),
            b"user edit sentinel"
        );
        assert_eq!(
            fs::read(backup.original_dir().join("api.dll")).unwrap(),
            b"pristine"
        );
        assert_eq!(
            backup.read_crack_inventory(APP_ID).unwrap(),
            vec!["api.dll"]
        );
    }

    #[test]
    fn failed_saved_copy_keeps_original_live_and_pending_removal_does_not_delete_it() {
        let (_temp, backup, game, staging) = setup();
        fs::write(game.join("api.dll"), b"pristine sentinel").unwrap();
        fs::write(staging.join("api.dll"), b"patch").unwrap();
        // A real rename failure, not a mock of the implementation.
        fs::create_dir(backup.crack_dir().join("api.dll")).unwrap();
        assert!(apply_staged_files(APP_ID, &staging, &game, &backup).is_err());
        assert_eq!(
            fs::read(game.join("api.dll")).unwrap(),
            b"pristine sentinel"
        );
        assert!(load_recovery(&backup, APP_ID).unwrap().files["api.dll"]
            .pending_sha256
            .is_some());
        assert_eq!(
            remove_applied_crack_files(APP_ID, &game, &backup).unwrap(),
            0
        );
        assert_eq!(
            fs::read(game.join("api.dll")).unwrap(),
            b"pristine sentinel"
        );
        assert_eq!(
            fs::read(backup.original_dir().join("api.dll")).unwrap(),
            b"pristine sentinel"
        );
        fs::remove_dir(backup.crack_dir().join("api.dll")).unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        assert_eq!(fs::read(game.join("api.dll")).unwrap(), b"patch");
    }

    #[test]
    fn inventory_save_failure_keeps_previous_prefix_and_does_not_delete_untouched_original() {
        let (_temp, backup, game, staging) = setup();
        fs::write(game.join("a.dll"), b"original a").unwrap();
        fs::write(staging.join("a.dll"), b"patch a").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        fs::remove_file(staging.join("a.dll")).unwrap();
        fs::write(game.join("b.dll"), b"original b").unwrap();
        fs::write(staging.join("b.dll"), b"patch b").unwrap();
        let blocked = PathBuf::from(format!(
            "{}.tmp",
            backup.crack_inventory_path(APP_ID).display()
        ));
        fs::create_dir(&blocked).unwrap();
        assert!(apply_staged_files(APP_ID, &staging, &game, &backup).is_err());
        assert_eq!(fs::read(game.join("a.dll")).unwrap(), b"patch a");
        assert_eq!(fs::read(game.join("b.dll")).unwrap(), b"original b");
        assert_eq!(load_recovery(&backup, APP_ID).unwrap().files.len(), 2);
        fs::remove_dir(blocked).unwrap();
        assert_eq!(
            remove_applied_crack_files(APP_ID, &game, &backup).unwrap(),
            1
        );
        assert_eq!(fs::read(game.join("b.dll")).unwrap(), b"original b");
        assert_eq!(
            fs::read(backup.original_dir().join("a.dll")).unwrap(),
            b"original a"
        );
    }

    #[test]
    fn immutable_publication_never_replaces_an_existing_original() {
        let (temp, _backup, _game, _staging) = setup();
        let source = temp.path().join("new.dll");
        let original = temp.path().join("original.dll");
        fs::write(&source, b"new patch").unwrap();
        fs::write(&original, b"first original").unwrap();
        assert!(atomic_copy(&source, &original, &hash_file(&source).unwrap(), false).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"first original");
        assert!(fs::read_dir(temp.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".aether-crack-")));
    }

    #[test]
    fn failed_atomic_publish_keeps_destination_sentinel_and_leaves_no_partial_live_file() {
        let (temp, _backup, _game, _staging) = setup();
        let source = temp.path().join("source.dll");
        let destination = temp.path().join("blocked.dll");
        fs::write(&source, b"complete patch").unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel.txt"), b"keep me").unwrap();
        assert!(atomic_copy(&source, &destination, &hash_file(&source).unwrap(), true).is_err());
        assert_eq!(
            fs::read(destination.join("sentinel.txt")).unwrap(),
            b"keep me"
        );
        assert_eq!(
            fs::read(temp.path().join("protected-sentinel.txt")).unwrap(),
            b"protected"
        );
    }

    #[test]
    fn legacy_inventory_is_merged_with_saved_files_before_a_new_import() {
        let (_temp, backup, game, staging) = setup();
        fs::write(game.join("a.dll"), b"old patch a").unwrap();
        fs::write(game.join("b.dll"), b"old patch b").unwrap();
        fs::write(backup.original_dir().join("a.dll"), b"original a").unwrap();
        fs::write(backup.crack_dir().join("a.dll"), b"old patch a").unwrap();
        fs::write(backup.crack_dir().join("b.dll"), b"old patch b").unwrap();
        fs::write(backup.crack_inventory_path(APP_ID), "b.dll\nb.dll\n").unwrap();
        fs::write(staging.join("c.dll"), b"new patch c").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        assert_eq!(
            backup.read_crack_inventory(APP_ID).unwrap(),
            vec!["a.dll", "b.dll", "c.dll"]
        );
        assert_eq!(
            remove_applied_crack_files(APP_ID, &game, &backup).unwrap(),
            3
        );
        assert_eq!(
            fs::read(backup.original_dir().join("a.dll")).unwrap(),
            b"original a"
        );
        assert!(!backup.original_dir().join("b.dll").exists());
    }

    #[test]
    fn flat_crack_still_replaces_all_matching_game_files() {
        let (_temp, backup, game, staging) = setup();
        for (directory, bytes) in [
            ("Binaries", b"original one".as_slice()),
            ("Engine", b"original two".as_slice()),
        ] {
            fs::create_dir_all(game.join(directory)).unwrap();
            fs::write(game.join(directory).join("api.dll"), bytes).unwrap();
        }
        fs::write(staging.join("api.dll"), b"patch").unwrap();
        let report = apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        assert_eq!(report.applied, 2);
        assert_eq!(
            fs::read(backup.original_dir().join("Binaries/api.dll")).unwrap(),
            b"original one"
        );
        assert_eq!(
            fs::read(backup.original_dir().join("Engine/api.dll")).unwrap(),
            b"original two"
        );
    }

    /// Steam's `installdir` can be a pure wrapper (`common/<name>/<name>/game.exe`).
    /// A crack file with no counterpart must land in the wrapped game root and be
    /// inventoried relative to it, so removal and saved-crack reapply keep working.
    #[test]
    fn counterpart_free_flat_file_lands_in_the_wrapped_game_root() {
        let (_temp, backup, game, staging) = setup();
        let inner = game.join("How to Fish");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("How to Fish.exe"), b"MZ fake").unwrap();
        fs::write(staging.join("OnlineFix.ini"), b"[fix]").unwrap();

        let report = apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();

        assert_eq!(report.files, vec!["How to Fish/OnlineFix.ini"]);
        assert_eq!(fs::read(inner.join("OnlineFix.ini")).unwrap(), b"[fix]");
        assert!(!game.join("OnlineFix.ini").exists());
        assert_eq!(
            backup.read_crack_inventory(APP_ID).unwrap(),
            vec!["How to Fish/OnlineFix.ini"]
        );
        assert_eq!(
            remove_applied_crack_files(APP_ID, &game, &backup).unwrap(),
            1
        );
        assert!(!inner.join("OnlineFix.ini").exists());
    }

    #[test]
    fn corrupt_or_unsafe_recovery_state_never_authorizes_deletion() {
        let (temp, backup, game, _staging) = setup();
        let sentinel = temp.path().join("outside-sentinel.txt");
        fs::write(&sentinel, b"outside protected").unwrap();
        fs::write(backup.crack_recovery_path(APP_ID), b"not JSON").unwrap();
        assert!(remove_applied_crack_files(APP_ID, &game, &backup).is_err());
        let mut state = RecoveryInventory {
            version: 1,
            files: BTreeMap::new(),
        };
        state.files.insert(
            "../outside-sentinel.txt".into(),
            RecoveryFile {
                had_original: false,
                original_sha256: None,
                applied_sha256: Some(hash_file(&sentinel).unwrap()),
                pending_sha256: None,
            },
        );
        fs::write(
            backup.crack_recovery_path(APP_ID),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert!(remove_applied_crack_files(APP_ID, &game, &backup).is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"outside protected");
    }

    #[test]
    fn backup_inside_game_is_refused_without_modifying_originals() {
        let (temp, _backup, game, staging) = setup();
        let root = game.join("backup");
        fs::create_dir_all(root.join("original")).unwrap();
        fs::create_dir_all(root.join("crack")).unwrap();
        let backup = GameBackup::for_tests(root);
        fs::write(game.join("api.dll"), b"original sentinel").unwrap();
        fs::write(staging.join("api.dll"), b"patch").unwrap();
        assert!(apply_staged_files(APP_ID, &staging, &game, &backup).is_err());
        assert_eq!(
            fs::read(game.join("api.dll")).unwrap(),
            b"original sentinel"
        );
        assert_eq!(
            fs::read(temp.path().join("protected-sentinel.txt")).unwrap(),
            b"protected"
        );
    }

    #[test]
    fn inventory_and_temporary_names_cannot_overwrite_game_original_backups() {
        for name in [
            format!("crack_{APP_ID}.txt"),
            format!("crack_{APP_ID}.txt.tmp"),
        ] {
            let (_temp, backup, game, staging) = setup();
            fs::write(game.join(&name), b"legitimate original sentinel").unwrap();
            fs::write(staging.join(&name), b"patch").unwrap();
            assert!(apply_staged_files(APP_ID, &staging, &game, &backup).is_err());
            assert_eq!(
                fs::read(game.join(&name)).unwrap(),
                b"legitimate original sentinel"
            );
            assert!(!backup.crack_recovery_path(APP_ID).exists());
        }
    }

    #[test]
    fn recovery_state_save_failure_occurs_before_any_live_change() {
        let (_temp, backup, game, staging) = setup();
        fs::write(game.join("api.dll"), b"pristine sentinel").unwrap();
        fs::write(staging.join("api.dll"), b"patch").unwrap();
        let blocked = PathBuf::from(format!(
            "{}.tmp",
            backup.crack_recovery_path(APP_ID).display()
        ));
        fs::create_dir(&blocked).unwrap();
        assert!(apply_staged_files(APP_ID, &staging, &game, &backup).is_err());
        assert_eq!(
            fs::read(game.join("api.dll")).unwrap(),
            b"pristine sentinel"
        );
        assert_eq!(
            fs::read(backup.original_dir().join("api.dll")).unwrap(),
            b"pristine sentinel"
        );
        assert!(!backup.crack_dir().join("api.dll").exists());
        fs::remove_dir(blocked).unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        assert_eq!(
            fs::read(backup.original_dir().join("api.dll")).unwrap(),
            b"pristine sentinel"
        );
    }

    #[cfg(windows)]
    #[test]
    fn case_variant_nested_imports_keep_original_absence_and_one_inventory_path() {
        let (_temp, backup, game, staging) = setup();
        fs::create_dir_all(game.join("Bin")).unwrap();
        fs::create_dir_all(staging.join("Bin")).unwrap();
        fs::write(staging.join("Bin/api.dll"), b"patch one").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        fs::remove_file(staging.join("Bin/api.dll")).unwrap();
        fs::write(staging.join("Bin/API.DLL"), b"patch two").unwrap();
        apply_staged_files(APP_ID, &staging, &game, &backup).unwrap();
        reapply_saved_crack_files(APP_ID, &game, &backup).unwrap();
        assert!(!backup.original_dir().join("Bin/api.dll").exists());
        assert_eq!(backup.read_crack_inventory(APP_ID).unwrap().len(), 1);
        assert_eq!(
            remove_applied_crack_files(APP_ID, &game, &backup).unwrap(),
            1
        );
        assert!(!game.join("Bin/api.dll").exists());
    }
}
