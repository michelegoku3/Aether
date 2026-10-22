//! Verified, retryable rollback of the UCOnline2 deployment journal.
//!
//! Completed entries are retired only after their filesystem action succeeds.
//! A private restore checkpoint bridges a successful rename and a failed
//! journal save, so a retry never deletes an original already restored.

use crate::external_tools::constants::UCO_DISABLED_SUFFIX;
use crate::external_tools::fs::{move_path, write_atomic};
use crate::online::deploy::{backup_dir_for, Journal, JournalEntry};
use crate::online::state::OnlineStateStore;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const RESTORE_CHECKPOINT: &str = "rollback_pending.json";

/// Text shown before the destructive, journal-less cleanup.
const CONFIRM_CLEANUP_MESSAGE: &str =
    "No recovery journal found. AetherDesk can remove the known UCOnline2 files, but some \
     legitimate game files may also be removed. You can easily restore them by verifying the \
     game file integrity.\n\nContinue?";

const VERIFY_INTEGRITY_STEPS: &str =
    "Steam > Library > right-click the game > Properties > Installed Files > Verify integrity of \
     game files";

/// Result of `disable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisableOutcome {
    /// The journal is gone and the cleanup is destructive: ask the user, then
    /// call again with `confirm_cleanup = true`.
    ConfirmationRequired { message: String },
    /// Files were restored (journal) or removed (confirmed cleanup).
    Done {
        message: String,
        warning: Option<String>,
    },
}

pub fn disable(
    app_id: u32,
    backup_root: &Path,
    state_path: &Path,
    game_root: Option<&Path>,
    confirm_cleanup: bool,
) -> Result<DisableOutcome, String> {
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
            return handle_missing_journal(
                app_id,
                &backup_dir,
                state_path,
                &mut store,
                game_root,
                confirm_cleanup,
            );
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
    Ok(DisableOutcome::Done {
        message: "Online disabled: journaled files restored and state cleared.".into(),
        warning: None,
    })
}

/// Journal-less path: either confirm the destructive cleanup or run it.
///
/// When the whole AetherDesk folder was deleted, journal, state and originals
/// are gone together. The only safe action left is removing the known UCO2
/// files, which may also hit legitimate files -> explicit confirmation and a
/// verify-integrity warning afterwards.
fn handle_missing_journal(
    app_id: u32,
    backup_dir: &Path,
    state_path: &Path,
    store: &mut OnlineStateStore,
    game_root: Option<&Path>,
    confirm_cleanup: bool,
) -> Result<DisableOutcome, String> {
    let uco2_present = game_root.is_some_and(|root| crate::online::foreign::scan(root).uco2);
    let backup_has_content = backup_dir.exists() && backup_dir_has_content(backup_dir)?;
    let record_exists = store.get(app_id).is_some();

    if uco2_present {
        if !confirm_cleanup {
            return Ok(DisableOutcome::ConfirmationRequired {
                message: CONFIRM_CLEANUP_MESSAGE.into(),
            });
        }
        let root = game_root.ok_or_else(|| {
            "The game folder is unknown; cannot remove UCOnline2 files safely.".to_string()
        })?;
        let removed = cleanup_orphaned_uco2(root)?;
        if record_exists {
            store.remove(app_id, state_path)?;
        }
        // Never delete originals: without the journal they cannot be mapped
        // back, so they stay available for manual recovery.
        let warning = cleanup_warning(
            "UCO2 files were removed.",
            &removed,
            backup_has_content.then_some(backup_dir),
        );
        return Ok(DisableOutcome::Done {
            message: "Online disabled: UCOnline2 files removed without a recovery journal.".into(),
            warning: Some(warning),
        });
    }

    if backup_has_content {
        return Err(format!(
            "Recovery journal is missing at {}; backups retained for manual recovery",
            backup_dir.join("journal.json").display()
        ));
    }

    if record_exists {
        store.remove(app_id, state_path)?;
        return Ok(DisableOutcome::Done {
            message: "No tracked UCOnline2 deployment to restore; stale state cleared.".into(),
            warning: None,
        });
    }

    // An empty leftover backup dir (e.g. from a failed enable) is not "state":
    // clean it and report the no-op instead of erroring.
    if backup_dir.exists() {
        let _ = fs::remove_dir(backup_dir);
    }
    Ok(DisableOutcome::Done {
        message: "No tracked UCOnline2 deployment to restore; no files changed.".into(),
        warning: None,
    })
}

/// Removes the files UCOnline2 can put in a game, using detection for the
/// exact destinations and the marker scan for non-standard layouts.
fn cleanup_orphaned_uco2(root: &Path) -> Result<Vec<PathBuf>, String> {
    let report = crate::online::foreign::scan(root);
    remove_targets(uco2_targets(root, &report))
}

/// Removes BOTH the online-fix.me crack files and the UCOnline2 files.
///
/// An online-fix.me drop overwrites the UCO2 steam_api/plugins, so the two are
/// never cleanly separable on disk: the journal-less "Clean" from the OFME
/// banner wipes both stacks and leaves the folder ready for a fresh choice.
pub(crate) fn cleanup_online_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let report = crate::online::foreign::scan(root);
    let mut targets = uco2_targets(root, &report);
    targets.extend(report.files);
    remove_targets(targets)
}

/// Files the UCOnline2 deployment owns: marker scan + the detected steam_api
/// destination + the overlay proxy target.
fn uco2_targets(root: &Path, report: &crate::online::foreign::ForeignOnlineReport) -> Vec<PathBuf> {
    let mut targets = report.uco2_files.clone();

    if let Ok(detection) = crate::online::detect::GameInspector::inspect(root) {
        if let Some(dir) = detection.steam_api_dir.as_deref() {
            targets.push(dir.join(detection.arch.steam_api_file_name()));
        }
        if let Ok(target) = crate::online::detect::overlay_target(&detection) {
            targets.push(target.path);
        }
    }

    targets
}

fn remove_targets(mut targets: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    targets.sort();
    targets.dedup();

    let mut removed = Vec::new();
    let mut parents = Vec::new();
    for path in targets {
        if !path.is_file() {
            continue;
        }
        fs::remove_file(&path).map_err(|e| format!("Remove {}: {e}", path.display()))?;
        if let Some(parent) = path.parent() {
            parents.push(parent.to_path_buf());
        }
        removed.push(path);
    }

    // Only the deployment's own `plugins/` dir: never a user data dir that
    // happens to share the name. `remove_dir` is a no-op unless it is empty.
    parents.sort();
    parents.dedup();
    for parent in parents {
        let is_plugins = parent
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("plugins"));
        if is_plugins {
            let _ = fs::remove_dir(&parent);
        }
    }

    Ok(removed)
}

fn cleanup_warning(prefix: &str, removed: &[PathBuf], backup_dir: Option<&Path>) -> String {
    let mut message = format!(
        "{prefix} Some legitimate game files may also be removed. You can easily restore them by \
         verifying the game file integrity: {VERIFY_INTEGRITY_STEPS}."
    );
    if let Some(dir) = backup_dir {
        message.push_str(&format!(
            " Original backups were kept at {}.",
            dir.display()
        ));
    }
    if !removed.is_empty() {
        let names = removed
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>()
            .join(", ");
        message.push_str(&format!(" Removed: {names}."));
    }
    message
}

/// Confirmation shown before the destructive "Clean" (OFME banner).
pub(crate) const CONFIRM_CLEAN_MESSAGE: &str =
    "Remove online-fix.me crack files? AetherDesk will delete the online-fix.me / SteamFix files \
     and any UCOnline2 files it detects in the game folder. Some legitimate game files may also be \
     removed: you can restore them by verifying the game file integrity.\n\nContinue?";

/// Journal-less cleanup from the OFME banner: remove the online-fix.me files
/// AND the UCOnline2 files, clear the UCO2 state, keep the backup originals and
/// report a verify-integrity warning. The saved crack is never touched.
///
/// Returns `(message, warning)`.
pub(crate) fn clean_online_conflicts(
    app_id: u32,
    backup_root: &Path,
    state_path: &Path,
    game_root: &Path,
) -> Result<(String, Option<String>), String> {
    let backup_dir = backup_dir_for(backup_root, app_id);
    let removed = cleanup_online_files(game_root)?;

    // UCO2 is gone: drop the record so the UI reports NotConfigured.
    if let Ok(content) = fs::read_to_string(state_path) {
        if let Ok(mut store) = serde_json::from_str::<OnlineStateStore>(&content) {
            if store.get(app_id).is_some() {
                store.remove(app_id, state_path)?;
            }
        }
    }
    // The journal can no longer be replayed (its files are gone); the original
    // backups stay on disk for manual recovery.
    remove_file_if_present(&backup_dir.join("journal.json"))?;
    remove_file_if_present(&backup_dir.join(RESTORE_CHECKPOINT))?;

    if removed.is_empty() {
        return Ok((
            "No online-fix.me or UCOnline2 files were found in the game folder.".into(),
            None,
        ));
    }

    let warning = cleanup_warning(
        "Crack and UCOnline2 files were removed.",
        &removed,
        backup_dir.exists().then_some(backup_dir.as_path()),
    );
    Ok((
        format!(
            "Removed {} online-fix.me / UCOnline2 file(s) from the game folder.",
            removed.len()
        ),
        Some(warning),
    ))
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

/// Reverts the entries a failed deploy recorded in memory, newest first.
///
/// Unlike [`disable`], this does not need the journal on disk: a deploy that
/// fails while saving the journal still has to put back the original it has
/// already moved. The caller owns the cleanup of the journal and backup dir.
pub(crate) fn rollback_partial(journal: &Journal, backup_dir: &Path) -> Result<(), String> {
    for entry in journal.entries.iter().rev() {
        match entry {
            JournalEntry::Deployed { path } => {
                remove_file_if_present(path)?;
                if let Some(parent) = path.parent().filter(|parent| {
                    parent
                        .file_name()
                        .is_some_and(|name| name.eq_ignore_ascii_case("plugins"))
                }) {
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
    }
    remove_file_if_present(&backup_dir.join(RESTORE_CHECKPOINT))?;
    let originals = backup_dir.join("original");
    match fs::remove_dir(&originals) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Original backups remain at {}; they were not restored: {error}",
                originals.display()
            ))
        }
    }
    Ok(())
}

/// Drops the journal and the now-empty backup directory left by a successful
/// automatic rollback, so a later Disable does not replay empty entries.
pub(crate) fn cleanup_rolled_back_backup(backup_dir: &Path) {
    let _ = remove_file_if_present(&backup_dir.join("journal.json"));
    let _ = remove_empty_dir(backup_dir);
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
    let expected = fingerprint(backup)?;
    if original.try_exists().map_err(|e| e.to_string())? {
        // A cross-volume move copies first and deletes second: if removal
        // crashed, the original already holds the restored bytes and only the
        // leftover backup must go. Idempotent retry instead of a false collision.
        if fingerprint(original)? == expected {
            remove_backup(backup)?;
            return Ok(());
        }
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
    let checkpoint =
        serde_json::to_vec(&(original, backup, &expected)).map_err(|e| e.to_string())?;
    write_atomic(&checkpoint_path, &checkpoint)?;
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    move_path(backup, original).map_err(|e| {
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

fn remove_backup(backup: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(backup).map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        fs::remove_dir_all(backup).map_err(|e| format!("Remove backup {}: {e}", backup.display()))
    } else {
        remove_file_if_present(backup)
    }
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
        let outcome = disable(4242, &backup_root, &state_path, Some(&game_root), false).unwrap();
        let DisableOutcome::Done { message, .. } = outcome else {
            panic!("expected Done");
        };
        assert!(message.contains("No tracked"), "got: {message}");
    }

    #[test]
    fn disable_cleans_an_empty_leftover_backup_dir() {
        let (_temp, backup_root, state_path, game_root) = empty_ctx();
        fs::create_dir_all(backup_dir_for(&backup_root, 4242)).unwrap();
        let outcome = disable(4242, &backup_root, &state_path, Some(&game_root), false).unwrap();
        let DisableOutcome::Done { message, .. } = outcome else {
            panic!("expected Done");
        };
        assert!(message.contains("No tracked"), "got: {message}");
        assert!(!backup_dir_for(&backup_root, 4242).exists());
    }

    #[test]
    fn disable_refuses_orphaned_backups_without_a_journal() {
        let (_temp, backup_root, state_path, game_root) = empty_ctx();
        let backup_dir = backup_dir_for(&backup_root, 4242);
        fs::create_dir_all(backup_dir.join("original")).unwrap();
        fs::write(backup_dir.join("original/game.dll"), b"original").unwrap();
        let error = disable(4242, &backup_root, &state_path, Some(&game_root), false).unwrap_err();
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

    /// Mini Unity game already carrying a UCOnline2 deployment.
    fn uco2_game_fixture(root: &Path) {
        fs::create_dir_all(root.join("MyGame_Data/Managed")).unwrap();
        fs::create_dir_all(root.join("MyGame_Data/Plugins/x86_64")).unwrap();
        fs::create_dir_all(root.join("plugins")).unwrap();
        fs::write(root.join("MyGame.exe"), b"MZfake").unwrap();
        fs::write(root.join("MyGame_Data/Managed/Assembly-CSharp.dll"), b"asm").unwrap();
        fs::write(
            root.join("MyGame_Data/Plugins/x86_64/steam_api64.dll"),
            b"uco2-x64",
        )
        .unwrap();
        fs::write(root.join("union-crax.ini"), b"[Settings]").unwrap();
        fs::write(root.join("plugins/photon_universal.dll"), b"photon").unwrap();
        fs::write(root.join("plugins/playfab_universal.dll"), b"playfab").unwrap();
        fs::write(root.join("plugins/original.dll"), b"original").unwrap();
        fs::write(root.join("version.dll"), b"overlay").unwrap();
    }

    #[test]
    fn missing_journal_requires_confirmation_before_cleaning() {
        let temp = tempfile::tempdir().unwrap();
        let game_root = temp.path().join("game");
        uco2_game_fixture(&game_root);
        let backup_root = temp.path().join("backup");
        let state_path = temp.path().join("state.json");

        let first = disable(4242, &backup_root, &state_path, Some(&game_root), false).unwrap();

        assert!(matches!(first, DisableOutcome::ConfirmationRequired { .. }));
        assert!(
            game_root.join("union-crax.ini").is_file(),
            "nothing may be deleted before the user confirms"
        );
    }

    #[test]
    fn confirmed_cleanup_removes_only_known_uco2_files() {
        let temp = tempfile::tempdir().unwrap();
        let game_root = temp.path().join("game");
        uco2_game_fixture(&game_root);
        let backup_root = temp.path().join("backup");
        let state_path = temp.path().join("state.json");

        let outcome = disable(4242, &backup_root, &state_path, Some(&game_root), true).unwrap();

        let DisableOutcome::Done { warning, .. } = outcome else {
            panic!("expected Done, got ConfirmationRequired");
        };
        assert!(
            warning.is_some(),
            "verify-integrity warning must be returned"
        );
        assert!(!game_root.join("union-crax.ini").exists());
        assert!(!game_root.join("plugins/photon_universal.dll").exists());
        assert!(!game_root.join("plugins/playfab_universal.dll").exists());
        assert!(!game_root
            .join("MyGame_Data/Plugins/x86_64/steam_api64.dll")
            .exists());
        assert!(!game_root.join("version.dll").exists());
        assert!(
            game_root.join("plugins/original.dll").is_file(),
            "unrelated files must survive"
        );
        assert!(
            game_root.join("plugins").is_dir(),
            "the plugins dir must stay when it is not empty"
        );
        assert!(!crate::online::foreign::scan(&game_root).uco2);
    }

    #[test]
    fn restore_backup_finishes_when_original_already_matches() {
        let temp = tempfile::tempdir().unwrap();
        let backup_dir = temp.path().join("backup");
        fs::create_dir_all(&backup_dir).unwrap();
        let original = temp.path().join("game.dll");
        let backup = backup_dir.join("game.dll");
        fs::write(&original, b"same bytes").unwrap();
        fs::write(&backup, b"same bytes").unwrap();

        restore_backup(&original, &backup, &backup_dir).unwrap();

        assert!(!backup.exists());
        assert_eq!(fs::read(&original).unwrap(), b"same bytes");
    }

    #[test]
    fn restore_backup_keeps_a_real_collision() {
        let temp = tempfile::tempdir().unwrap();
        let backup_dir = temp.path().join("backup");
        fs::create_dir_all(&backup_dir).unwrap();
        let original = temp.path().join("game.dll");
        let backup = backup_dir.join("game.dll");
        fs::write(&original, b"user file").unwrap();
        fs::write(&backup, b"original").unwrap();

        let error = restore_backup(&original, &backup, &backup_dir).unwrap_err();

        assert!(error.contains("collision"), "got: {error}");
        assert_eq!(fs::read(&original).unwrap(), b"user file");
        assert!(backup.is_file());
    }

    /// Game with both an online-fix.me crack and a UCOnline2 deploy on disk.
    fn mixed_online_fixture(root: &Path) {
        fs::create_dir_all(root.join("MyGame_Data/Managed")).unwrap();
        fs::create_dir_all(root.join("MyGame_Data/Plugins/x86_64")).unwrap();
        fs::create_dir_all(root.join("plugins")).unwrap();
        fs::write(root.join("MyGame.exe"), b"MZfake").unwrap();
        fs::write(root.join("MyGame_Data/Managed/Assembly-CSharp.dll"), b"asm").unwrap();
        fs::write(
            root.join("MyGame_Data/Plugins/x86_64/steam_api64.dll"),
            b"uco2-x64",
        )
        .unwrap();
        fs::write(root.join("union-crax.ini"), b"[Settings]").unwrap();
        fs::write(root.join("plugins/photon_universal.dll"), b"photon").unwrap();
        fs::write(root.join("version.dll"), b"overlay").unwrap();
        fs::write(root.join("OnlineFix.ini"), b"[fix]").unwrap();
        fs::write(root.join("OnlineFix64.dll"), b"ofme").unwrap();
        fs::write(root.join("dlllist.txt"), b"list").unwrap();
        fs::write(root.join("plugins/original.dll"), b"original").unwrap();
    }

    #[test]
    fn clean_online_conflicts_removes_uco2_and_ofme_but_keeps_legit_and_backups() {
        let temp = tempfile::tempdir().unwrap();
        let game_root = temp.path().join("game");
        mixed_online_fixture(&game_root);
        let backup_root = temp.path().join("backup");
        let backup_dir = backup_dir_for(&backup_root, 4242);
        fs::create_dir_all(backup_dir.join("original")).unwrap();
        fs::write(backup_dir.join("original/steam_api64.dll"), b"original-dll").unwrap();
        let state_path = temp.path().join("state.json");

        let (message, warning) =
            clean_online_conflicts(4242, &backup_root, &state_path, &game_root).unwrap();

        assert!(message.contains("Removed"), "got: {message}");
        assert!(warning.is_some(), "verify-integrity warning expected");
        // Both stacks are gone.
        assert!(!game_root.join("union-crax.ini").exists());
        assert!(!game_root.join("plugins/photon_universal.dll").exists());
        assert!(!game_root.join("version.dll").exists());
        assert!(!game_root
            .join("MyGame_Data/Plugins/x86_64/steam_api64.dll")
            .exists());
        assert!(!game_root.join("OnlineFix.ini").exists());
        assert!(!game_root.join("OnlineFix64.dll").exists());
        assert!(!game_root.join("dlllist.txt").exists());
        // Legit files and the backup originals survive.
        assert!(game_root.join("MyGame.exe").is_file());
        assert!(game_root.join("plugins/original.dll").is_file());
        assert!(backup_dir.join("original/steam_api64.dll").is_file());
    }

    #[test]
    fn clean_online_conflicts_reports_a_noop_when_nothing_is_detected() {
        let temp = tempfile::tempdir().unwrap();
        let game_root = temp.path().join("game");
        fs::create_dir_all(&game_root).unwrap();
        fs::write(game_root.join("Game.exe"), b"MZ").unwrap();
        let backup_root = temp.path().join("backup");
        let state_path = temp.path().join("state.json");

        let (message, warning) =
            clean_online_conflicts(4242, &backup_root, &state_path, &game_root).unwrap();

        assert!(message.contains("No "), "got: {message}");
        assert!(warning.is_none());
    }
}
