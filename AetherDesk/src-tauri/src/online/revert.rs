//! Verified, retryable rollback of the UCOnline2 deployment journal.
//!
//! Completed entries are retired only after their filesystem action succeeds.
//! A private restore checkpoint bridges a successful rename and a failed
//! journal save, so a retry never deletes an original already restored.

use crate::external_tools::constants::UCO_DISABLED_SUFFIX;
use crate::external_tools::fs::write_atomic;
use crate::online::deploy::{backup_dir_for, Journal, JournalEntry};
use crate::online::state::OnlineStateStore;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const RESTORE_CHECKPOINT: &str = "rollback_pending.json";

pub fn disable(
    app_id: u32,
    backup_root: &Path,
    state_path: &Path,
    game_root: Option<&Path>,
) -> Result<String, String> {
    let backup_dir = backup_dir_for(backup_root, app_id);
    // Do not turn a corrupt/unreadable recovery index into an empty store.
    let mut store: OnlineStateStore = match fs::read_to_string(state_path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("Invalid online state {}: {e}", state_path.display()))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => OnlineStateStore::default(),
        Err(error) => {
            return Err(format!(
                "Read online state {}: {error}",
                state_path.display()
            ))
        }
    };
    let journal_path = backup_dir.join("journal.json");
    let mut journal: Journal = match fs::read_to_string(&journal_path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("Invalid recovery journal {}: {e}", journal_path.display()))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if store.get(app_id).is_some() {
                // A state record without its journal: originals may be orphaned
                // in the backup dir — never guess, manual recovery it is.
                return Err(format!(
                    "Recovery journal is missing at {}; state and backups retained for manual recovery",
                    journal_path.display()
                ));
            }
            if game_root.is_some_and(|root| crate::online::foreign::scan(root).uco2) {
                return Err("External UCOnline2 files have no recovery journal; automatic restoration is unsafe".into());
            }
            // An empty leftover backup dir (e.g. from a failed enable) is not
            // "state": clean it and report the no-op instead of erroring.
            if backup_dir.exists() && !backup_dir_has_content(&backup_dir)? {
                let _ = fs::remove_dir(&backup_dir);
            } else if backup_dir.exists() {
                return Err(format!(
                    "Recovery journal is missing at {}; backups retained for manual recovery",
                    journal_path.display()
                ));
            }
            return Ok("No tracked UCOnline2 deployment to restore; no files changed.".into());
        }
        Err(error) => {
            return Err(format!(
                "Read recovery journal {}: {error}",
                journal_path.display()
            ))
        }
    };

    revert_from_journal(&mut journal, &backup_dir)?;
    // The now-empty journal stays available if persisting the index fails.
    if store.get(app_id).is_some() {
        store.remove(app_id, state_path)?;
    }
    remove_file_if_present(&journal_path)?;
    remove_empty_dir(&backup_dir)?;
    Ok("Online disabled: journaled files restored and state cleared.".into())
}

/// True when the backup dir holds anything worth retaining (originals,
/// journals, checkpoints). An existing-but-empty dir is just litter.
fn backup_dir_has_content(backup_dir: &Path) -> Result<bool, String> {
    match fs::read_dir(backup_dir) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Read backup directory {}: {error}",
            backup_dir.display()
        )),
    }
}

fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Remove {}: {error}", path.display())),
    }
}

fn remove_empty_dir(path: &Path) -> Result<(), String> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "Remove empty directory {}: {error}",
            path.display()
        )),
    }
}

fn revert_from_journal(journal: &mut Journal, backup_dir: &Path) -> Result<(), String> {
    while let Some(entry) = journal.entries.last() {
        match entry {
            JournalEntry::Deployed { path } => {
                remove_file_if_present(path)?;
                if let Some(parent) = path.parent().filter(|parent| {
                    parent
                        .file_name()
                        .is_some_and(|name| name.eq_ignore_ascii_case("plugins"))
                }) {
                    // Remove owned empty deployment directories BEFORE restoring originals.
                    remove_empty_dir(parent)?;
                }
            }
            JournalEntry::Neutralized { path } => {
                let disabled = path.with_file_name(format!(
                    "{}{}",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
                    UCO_DISABLED_SUFFIX
                ));
                restore_backup(path, &disabled, backup_dir)?;
            }
            JournalEntry::BackedUp { original, backup } => {
                restore_backup(original, backup, backup_dir)?;
            }
        }
        let _ = journal.entries.pop();
        journal.save(backup_dir)?;
        remove_file_if_present(&backup_dir.join(RESTORE_CHECKPOINT))?;
    }
    remove_file_if_present(&backup_dir.join(RESTORE_CHECKPOINT))?;
    // Never discard originals not accounted for by the journal (e.g. a failed enable).
    let originals = backup_dir.join("original");
    match fs::remove_dir(&originals) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Original backups remain at {}; recovery state retained: {error}",
                originals.display()
            ))
        }
    }
    Ok(())
}

fn restore_backup(original: &Path, backup: &Path, backup_dir: &Path) -> Result<(), String> {
    let checkpoint_path = backup_dir.join(RESTORE_CHECKPOINT);
    if !backup.try_exists().map_err(|e| e.to_string())? {
        // A previous rename may have succeeded just before journal.save failed.
        let content = fs::read_to_string(&checkpoint_path).map_err(|e| {
            format!(
                "Missing backup {}; cannot prove restoration: {e}",
                backup.display()
            )
        })?;
        let (saved_original, saved_backup, expected): (PathBuf, PathBuf, String) =
            serde_json::from_str(&content).map_err(|e| e.to_string())?;
        if saved_original == original
            && saved_backup == backup
            && fingerprint(original)? == expected
        {
            return Ok(());
        }
        return Err(format!(
            "Missing backup {}; restored content not verified",
            backup.display()
        ));
    }
    if original.try_exists().map_err(|e| e.to_string())? {
        // A deployed plugins directory may exist even when a plugin copy failed.
        // Only an EMPTY directory can be removed; never overwrite user collisions.
        let metadata = fs::symlink_metadata(original).map_err(|e| e.to_string())?;
        let linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            linked || metadata.file_attributes() & 0x400 != 0
        };
        if !linked && backup.is_dir() && metadata.is_dir() {
            fs::remove_dir(original).map_err(|e| {
                format!(
                    "Restore collision at {}; backup retained: {e}",
                    original.display()
                )
            })?;
        } else {
            return Err(format!(
                "Restore collision at {}; backup retained at {}",
                original.display(),
                backup.display()
            ));
        }
    }
    let expected = fingerprint(backup)?;
    let checkpoint =
        serde_json::to_vec(&(original, backup, &expected)).map_err(|e| e.to_string())?;
    write_atomic(&checkpoint_path, &checkpoint)?;
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::rename(backup, original).map_err(|e| {
        format!(
            "Restore {} from {}: {e}",
            original.display(),
            backup.display()
        )
    })?;
    if fingerprint(original)? != expected {
        return Err(format!(
            "Restored content differs at {}; checkpoint retained",
            original.display()
        ));
    }
    Ok(())
}

fn fingerprint(path: &Path) -> Result<String, String> {
    fn visit(path: &Path, digest: &mut Sha256) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            linked || metadata.file_attributes() & 0x400 != 0
        };
        if linked {
            return Err(format!(
                "Cannot fingerprint linked backup {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            digest.update(b"D");
            let mut entries = fs::read_dir(path)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            entries.sort_by_key(fs::DirEntry::file_name);
            // Encode tree boundaries, not just preorder names and contents.
            let count = u64::try_from(entries.len()).map_err(|e| e.to_string())?;
            digest.update(count.to_le_bytes());
            for entry in entries {
                let name = entry.file_name();
                let bytes = name.as_encoded_bytes();
                let length = u64::try_from(bytes.len()).map_err(|e| e.to_string())?;
                digest.update(length.to_le_bytes());
                digest.update(bytes);
                visit(&entry.path(), digest)?;
            }
        } else if metadata.is_file() {
            digest.update(b"F");
            digest.update(metadata.len().to_le_bytes());
            let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
            let mut buffer = [0_u8; 16 * 1024];
            loop {
                let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
        } else {
            return Err(format!(
                "Cannot fingerprint special file {}",
                path.display()
            ));
        }
        Ok(())
    }
    let mut digest = Sha256::new();
    visit(path, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online::deploy::backup_dir_for;

    fn empty_ctx() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let backup_root = temp.path().join("backup");
        let state_path = temp.path().join("uc_online2.json");
        let game_root = temp.path().join("game");
        fs::create_dir_all(&game_root).unwrap();
        (temp, backup_root, state_path, game_root)
    }

    #[test]
    fn disable_without_any_state_is_a_noop_not_an_error() {
        let (_temp, backup_root, state_path, game_root) = empty_ctx();
        let message = disable(4242, &backup_root, &state_path, Some(&game_root)).unwrap();
        assert!(message.contains("No tracked"), "got: {message}");
    }

    #[test]
    fn disable_cleans_an_empty_leftover_backup_dir() {
        let (_temp, backup_root, state_path, game_root) = empty_ctx();
        fs::create_dir_all(backup_dir_for(&backup_root, 4242)).unwrap();
        let message = disable(4242, &backup_root, &state_path, Some(&game_root)).unwrap();
        assert!(message.contains("No tracked"), "got: {message}");
        assert!(!backup_dir_for(&backup_root, 4242).exists());
    }

    #[test]
    fn disable_refuses_orphaned_backups_without_a_journal() {
        let (_temp, backup_root, state_path, game_root) = empty_ctx();
        let backup_dir = backup_dir_for(&backup_root, 4242);
        fs::create_dir_all(backup_dir.join("original")).unwrap();
        fs::write(backup_dir.join("original/game.dll"), b"original").unwrap();
        let error = disable(4242, &backup_root, &state_path, Some(&game_root)).unwrap_err();
        assert!(error.contains("manual recovery"), "got: {error}");
        // Nothing was touched: the orphan is still there for manual recovery.
        assert!(backup_dir.join("original/game.dll").is_file());
    }

    #[test]
    fn recovery_fingerprint_distinguishes_sibling_from_nested_file() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for root in [&first, &second] {
            fs::create_dir_all(root.join("a")).unwrap();
            fs::write(root.join("a/b"), b"b sentinel").unwrap();
        }
        fs::write(first.join("c"), b"c sentinel").unwrap();
        fs::write(second.join("a/c"), b"c sentinel").unwrap();
        assert_ne!(fingerprint(&first).unwrap(), fingerprint(&second).unwrap());
        assert_eq!(fs::read(first.join("c")).unwrap(), b"c sentinel");
    }
}
