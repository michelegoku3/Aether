//! The Lua backup has two destinations and they are not interchangeable:
//! `backup/<app_id>/lua/<app_id>.lua` keeps the PRISTINE version (the first
//! Lua seen or the last one downloaded from the sources) and every other
//! version is archived in `backup/<app_id>/lua/history/`.
//!
//! Desk-modified Lua (a hand edit, the update toggle rewriting the pins) must
//! land in the history only: replacing the original would put the modified
//! file back where a restore or an uninstall expects the pristine one.

use crate::core::backup::{GameBackup, LUA_HISTORY_SUBDIR};
use crate::manifest::package::ManifestPackageFile;

fn setup() -> (tempfile::TempDir, GameBackup) {
    let dir = tempfile::tempdir().expect("tempdir");
    let backup = GameBackup::for_tests(dir.path().to_path_buf());
    (dir, backup)
}

/// Content of the pristine slot (`lua/<app_id>.lua`), when it exists.
fn original_lua(backup: &GameBackup, app_id: u32) -> Option<String> {
    std::fs::read_to_string(backup.lua_dir().join(format!("{app_id}.lua"))).ok()
}

/// True when `content` is archived in the version history.
fn history_contains(backup: &GameBackup, content: &str) -> bool {
    let history_dir = backup.lua_dir().join(LUA_HISTORY_SUBDIR);
    std::fs::read_dir(history_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.path().extension().and_then(|ext| ext.to_str()) == Some("lua")
                })
                .any(|entry| {
                    std::fs::read_to_string(entry.path()).is_ok_and(|text| text == content)
                })
        })
        .unwrap_or(false)
}

#[test]
fn a_desk_modified_lua_never_replaces_the_pristine_original() {
    let (_dir, backup) = setup();
    backup.backup_lua_artifacts(480, "PRISTINE", &[]).unwrap();
    backup.backup_lua_variant(480, "USER EDIT", &[]).unwrap();

    assert_eq!(original_lua(&backup, 480).as_deref(), Some("PRISTINE"));
    assert!(history_contains(&backup, "USER EDIT"));
}

#[test]
fn a_pristine_download_still_replaces_the_original() {
    let (_dir, backup) = setup();
    backup.backup_lua_artifacts(480, "PRISTINE", &[]).unwrap();
    backup
        .backup_lua_artifacts(480, "NEWER DOWNLOAD", &[])
        .unwrap();

    assert_eq!(
        original_lua(&backup, 480).as_deref(),
        Some("NEWER DOWNLOAD")
    );
    // The replaced version stays available in the history.
    assert!(history_contains(&backup, "PRISTINE"));
}

#[test]
fn a_variant_keeps_the_referenced_manifests_in_the_backup() {
    let (_dir, backup) = setup();
    backup.backup_lua_artifacts(480, "PRISTINE", &[]).unwrap();
    let manifests = vec![ManifestPackageFile {
        file_name: "1_2.manifest".to_string(),
        bytes: b"manifest-bytes".to_vec(),
    }];
    backup
        .backup_lua_variant(480, "USER EDIT", &manifests)
        .unwrap();

    let copied = std::fs::read(backup.lua_dir().join("1_2.manifest")).unwrap();
    assert_eq!(copied, b"manifest-bytes");
}
