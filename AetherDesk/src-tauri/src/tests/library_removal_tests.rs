//! Removing a game's Lua from the Library is not an uninstall.
//!
//! The Lua is the DLL's ownership/depot trigger, while Steam's `appmanifest`
//! is what makes a game "installed". A legitimate owner who installed the Lua
//! to unlock DLC must be able to delete it and go back to the base game, so an
//! installed app is exactly the case this action exists for. The only state
//! that still refuses is Steam actively writing the appmanifest: the file is
//! being replaced and the removal would race it.

use crate::core::backup::GameBackup;
use crate::core::library_removal::remove_lua_from_library;
use std::fs;
use std::path::Path;

fn steam_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp steam root");
    fs::create_dir_all(dir.path().join("config").join("stplug-in")).expect("stplug-in");
    fs::create_dir_all(dir.path().join("steamapps").join("common")).expect("steamapps/common");
    fs::write(dir.path().join("steam.exe"), b"").expect("steam.exe");
    dir
}

fn data_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp AetherData root")
}

fn plugin_dir(root: &Path) -> std::path::PathBuf {
    root.join("config").join("stplug-in")
}

fn write_lua(root: &Path, app_id: u32, body: &str) {
    fs::write(plugin_dir(root).join(format!("{app_id}.lua")), body).expect("write lua");
}

fn write_acf(root: &Path, app_id: u32, state_flags: &str) {
    let body = format!(
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"Game {app_id}\"\n\t\"StateFlags\"\t\t\"{state_flags}\"\n\t\"installdir\"\t\t\"Game {app_id}\"\n\t\"buildid\"\t\t\"1\"\n}}\n"
    );
    fs::write(
        root.join("steamapps")
            .join(format!("appmanifest_{app_id}.acf")),
        body,
    )
    .expect("write acf");
}

fn remove(
    steam: &tempfile::TempDir,
    data: &tempfile::TempDir,
    app_id: u32,
) -> Result<crate::core::library_removal::LuaRemovalReport, String> {
    let backup = GameBackup::for_tests(data.path().to_path_buf());
    remove_lua_from_library(&steam.path().display().to_string(), app_id, Some(&backup))
}

#[test]
fn removes_the_lua_of_a_fully_installed_game_and_archives_it() {
    let steam = steam_root();
    let data = data_root();
    write_lua(steam.path(), 730, "-- Counter-Strike 2\n");
    // 0x4: Steam's "Fully Installed" bit.
    write_acf(steam.path(), 730, "4");

    let report = remove(&steam, &data, 730).expect("an installed game can be removed");

    assert!(report.lua_removed);
    assert!(report.archived, "the deleted Lua must stay restorable");
    assert!(!plugin_dir(steam.path()).join("730.lua").exists());
    let backup = GameBackup::for_tests(data.path().to_path_buf());
    assert!(backup.has_lua_version(730, b"-- Counter-Strike 2\n"));
}

#[test]
fn refuses_while_steam_is_still_writing_the_game() {
    let steam = steam_root();
    let data = data_root();
    write_lua(steam.path(), 700, "-- Game 700\n");
    // 0x402: Update Required + Update Started, Steam is writing.
    write_acf(steam.path(), 700, "1026");

    let error = remove(&steam, &data, 700).expect_err("a running update must not lose its Lua");

    assert!(error.contains("still writing"), "unexpected error: {error}");
    assert!(plugin_dir(steam.path()).join("700.lua").exists());
}

#[test]
fn a_paused_update_still_allows_removal() {
    let steam = steam_root();
    let data = data_root();
    write_lua(steam.path(), 701, "-- Game 701\n");
    // 0x202: Update Required + Update Paused, the content is already complete.
    write_acf(steam.path(), 701, "514");

    let report = remove(&steam, &data, 701).expect("a paused update is not a write");

    assert!(report.lua_removed);
}

#[test]
fn removes_the_stale_lua_bak_with_the_lua() {
    let steam = steam_root();
    let data = data_root();
    write_lua(steam.path(), 702, "-- Game 702\n");
    fs::write(
        plugin_dir(steam.path()).join("702.lua.bak"),
        b"-- old copy\n",
    )
    .expect("write bak");

    let report = remove(&steam, &data, 702).expect("remove");

    assert!(report.lua_removed);
    assert!(report.backup_removed);
    assert!(!plugin_dir(steam.path()).join("702.lua.bak").exists());
}

#[test]
fn a_missing_lua_is_reported_not_an_error() {
    let steam = steam_root();
    let data = data_root();

    let report = remove(&steam, &data, 703).expect("an absent Lua is not a failure");

    assert!(!report.lua_removed);
    assert!(!report.archived);
    assert!(!report.backup_removed);
}
