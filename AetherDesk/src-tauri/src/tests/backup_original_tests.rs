//! The Lua backup has two destinations and they are not interchangeable:
//! `backup/<app_id>/lua/<app_id>.lua` keeps the PRISTINE version (the first
//! Lua seen or the last one downloaded from the sources) and every other
//! version is archived in `backup/<app_id>/lua/history/`.
//!
//! Desk-modified Lua (a hand edit, the update toggle rewriting the pins) must
//! land in the history only: replacing the original would put the modified
//! file back where a restore or an uninstall expects the pristine one.

use crate::application::store::installation::apply_update_policy_and_backup_with;
use crate::core::backup::{GameBackup, LUA_HISTORY_SUBDIR};
use crate::manifest::package::ManifestPackageFile;
use std::path::Path;

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

/// Number of archived versions in the history.
fn history_entries(backup: &GameBackup) -> usize {
    let history_dir = backup.lua_dir().join(LUA_HISTORY_SUBDIR);
    std::fs::read_dir(history_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.path().extension().and_then(|ext| ext.to_str()) == Some("lua")
                })
                .count()
        })
        .unwrap_or(0)
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

/// The Lua a store install publishes: one locked depot (an active
/// `setManifestid`) and one depot Steam may update (a commented one).
const PACKAGE_LUA: &str = r#"-- MAIN APPLICATION
addappid(3618390, 1, "basekey") -- Game

-- MAIN APP DEPOTS
addappid(3618391, 1, "key1") -- Windows
setManifestid(3618391, "1111111111111111111")
addappid(3618392, 1, "key2") -- Linux
-- setManifestid(3618392, "2222222222222222222")
"#;

/// The content the update policy produces: enabling updates comments the
/// locked depot, so Steam may update it. The real editor drives it, so the
/// rewrite is the one the store install performs.
fn policy_rewritten_lua() -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let stplug = dir.path().join("config").join("stplug-in");
    std::fs::create_dir_all(&stplug).expect("stplug-in dir");
    std::fs::write(stplug.join("3618390.lua"), PACKAGE_LUA).expect("write lua");
    let pins = crate::manifest::pins::LuaManifestPins::new(dir.path(), 3618390);
    pins.set_updates_enabled(true).expect("enable updates");
    std::fs::read_to_string(pins.lua_path()).expect("read lua")
}

/// A store install applies the update policy after publishing the package.
/// The policy rewrite is Desk-modified: the package content stays the
/// pristine one and the rewritten content is archived in the history.
#[test]
fn an_install_policy_rewrite_keeps_the_package_content_in_the_original_slot() {
    let (_dir, backup) = setup();
    let policy_lua = policy_rewritten_lua();
    assert_ne!(policy_lua, PACKAGE_LUA);

    backup
        .backup_installed_lua(480, PACKAGE_LUA, &policy_lua, &[])
        .unwrap();

    assert_eq!(original_lua(&backup, 480).as_deref(), Some(PACKAGE_LUA));
    assert!(history_contains(&backup, &policy_lua));
}

/// An install whose policy changes nothing has no variant to archive: the
/// history stays empty and the original holds the published content.
#[test]
fn an_install_without_a_policy_rewrite_archives_no_variant() {
    let (_dir, backup) = setup();

    backup
        .backup_installed_lua(480, PACKAGE_LUA, PACKAGE_LUA, &[])
        .unwrap();

    assert_eq!(original_lua(&backup, 480).as_deref(), Some(PACKAGE_LUA));
    assert!(!history_contains(&backup, PACKAGE_LUA));
}

/// A Steam root whose `config/stplug-in/<app_id>.lua` holds the Lua an
/// install published. `steam.exe` makes the root pass the validation the
/// production read applies.
fn steam_root_with_installed_lua(app_id: u32, lua: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("steam.exe"), b"").expect("steam.exe");
    let stplug = dir.path().join("config").join("stplug-in");
    std::fs::create_dir_all(&stplug).expect("stplug-in dir");
    std::fs::write(stplug.join(format!("{app_id}.lua")), lua).expect("write lua");
    let steam_path = dir.path().to_string_lossy().to_string();
    (dir, steam_path)
}

/// The install-time backup reads the installed Lua before the update policy
/// rewrites it. Reading it only afterwards would put the rewritten content in
/// the pristine slot, and the history would stay empty.
#[test]
fn the_install_backup_reads_the_lua_before_the_update_policy() {
    let (_steam, steam_path) = steam_root_with_installed_lua(480, PACKAGE_LUA);
    let stplug = Path::new(&steam_path).join("config").join("stplug-in");
    let (_dir, backup) = setup();

    apply_update_policy_and_backup_with(&backup, true, 480, &steam_path, PACKAGE_LUA, &[]).unwrap();

    // The pristine slot holds the content the install published.
    assert_eq!(original_lua(&backup, 480).as_deref(), Some(PACKAGE_LUA));
    // The policy rewrite is a variant and is archived in the history.
    let rewritten = std::fs::read_to_string(stplug.join("480.lua")).expect("read lua");
    assert_ne!(rewritten, PACKAGE_LUA);
    assert!(history_contains(&backup, &rewritten));

    // The next install publishes the package Lua again; with the policy off
    // nothing is rewritten, so the pristine content stays and the history
    // does not grow.
    std::fs::write(stplug.join("480.lua"), PACKAGE_LUA).expect("write lua");
    let history_before = history_entries(&backup);

    apply_update_policy_and_backup_with(&backup, false, 480, &steam_path, PACKAGE_LUA, &[])
        .unwrap();

    assert_eq!(original_lua(&backup, 480).as_deref(), Some(PACKAGE_LUA));
    assert_eq!(history_entries(&backup), history_before);
}
