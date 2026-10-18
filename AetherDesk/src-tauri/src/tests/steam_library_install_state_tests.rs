//! `installed` vs `installing` is Steam's own statement, not a guess.
//!
//! The AppState `StateFlags` "Fully Installed" bit (0x4) is set only when Steam
//! has finished writing the content; the "Update Required" bit (0x2) marks an
//! install/update that Steam has not completed. The game folder is deliberately
//! NOT evidence: Steam leaves `steamapps/common/<installdir>` behind (empty, or
//! holding leftover crack files) after an uninstall.

use crate::steam::library::SteamLibraryScanner;
use std::fs;
use std::path::Path;

fn steam_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp steam root");
    fs::create_dir_all(dir.path().join("config").join("stplug-in")).expect("stplug-in");
    fs::create_dir_all(dir.path().join("steamapps").join("common")).expect("steamapps/common");
    dir
}

fn write_lua(root: &Path, app_id: u32) {
    fs::write(
        root.join("config")
            .join("stplug-in")
            .join(format!("{app_id}.lua")),
        format!("-- Game {app_id}\naddappid({app_id}, 1, \"key\")\n"),
    )
    .expect("write lua");
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

/// `(installed, installing)` for one App ID, straight from the Library scan.
fn install_state(root: &Path, app_id: u32) -> (bool, bool) {
    let game = SteamLibraryScanner::new(root)
        .scan_installed_games()
        .into_iter()
        .find(|game| game.id == app_id)
        .expect("the Lua game is listed");
    (game.installed, game.installing)
}

#[test]
fn a_queued_install_is_installing_not_installed() {
    let root = steam_root();
    write_lua(root.path(), 700);
    // Update Required + Update Started: Steam is writing, nothing to play yet.
    write_acf(root.path(), 700, "1026");

    assert_eq!(install_state(root.path(), 700), (false, true));
}

#[test]
fn a_completed_install_is_installed_and_not_installing() {
    let root = steam_root();
    write_lua(root.path(), 701);
    write_acf(root.path(), 701, "4");

    assert_eq!(install_state(root.path(), 701), (true, false));
}

#[test]
fn an_update_required_but_content_complete_game_stays_installed() {
    let root = steam_root();
    write_lua(root.path(), 702);
    // Fully Installed + Update Required: Steam wants an update, the files are
    // there. This is also the resting state of an Aether-pinned version.
    write_acf(root.path(), 702, "6");

    assert_eq!(install_state(root.path(), 702), (true, false));
}

#[test]
fn an_unknown_state_flags_value_does_not_hide_an_installed_game() {
    let root = steam_root();
    write_lua(root.path(), 703);
    write_acf(root.path(), 703, "0");

    assert_eq!(install_state(root.path(), 703), (true, false));
}

#[test]
fn a_lua_only_game_is_neither_installed_nor_installing() {
    let root = steam_root();
    write_lua(root.path(), 704);

    assert_eq!(install_state(root.path(), 704), (false, false));
}
