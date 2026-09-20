//! The Library scanner is the only place that reads every managed Lua anyway,
//! so it is also where a broken pin is detected (F4): the diagnosis travels
//! with the game card, and the version editor can then open and repair it.

use crate::steam::library::SteamLibraryScanner;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tempdir() -> TempDirGuard {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let base = std::env::temp_dir().join(format!(
        "aether_steam_library_test_{}_{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("config").join("stplug-in")).expect("create stplug-in");
    TempDirGuard(base)
}

fn write_lua(tmp: &TempDirGuard, app_id: u32, content: &str) {
    fs::write(
        tmp.path()
            .join("config")
            .join("stplug-in")
            .join(format!("{app_id}.lua")),
        content,
    )
    .expect("write lua");
}

#[test]
fn scan_flags_a_game_whose_lua_cannot_be_loaded() {
    let tmp = tempdir();
    write_lua(
        &tmp,
        3558400,
        "-- Backseat Drivers\naddappid(3558401, 1, \"key\")\nsetManifestid(3558401, 0x0484FB)\n",
    );
    write_lua(
        &tmp,
        3618390,
        "-- Healthy Game\naddappid(3618391, 1, \"key\")\nsetManifestid(3618391, \"1234\")\n",
    );

    let games = SteamLibraryScanner::new(tmp.path()).scan_installed_games();
    assert_eq!(games.len(), 2);

    let broken = games
        .iter()
        .find(|game| game.id == 3558400)
        .expect("broken game present");
    assert_eq!(broken.lua_issues.len(), 1, "one malformed pin");
    let issue = &broken.lua_issues[0];
    assert_eq!(issue.line, 3);
    assert_eq!(issue.depot_id, Some(3558401));
    assert!(issue.active, "the call is executed, so the file is rejected");

    let healthy = games
        .iter()
        .find(|game| game.id == 3618390)
        .expect("healthy game present");
    assert!(
        healthy.lua_issues.is_empty(),
        "a healthy Lua must stay unmarked"
    );
}

#[test]
fn a_commented_malformed_pin_does_not_break_the_current_load() {
    let tmp = tempdir();
    write_lua(
        &tmp,
        3558400,
        "-- Locked Game\naddappid(3558401, 1, \"key\")\nsetManifestid(3558401, \"1234\")\n--setManifestid(3558401, 0x0484FB)\n",
    );

    let games = SteamLibraryScanner::new(tmp.path()).scan_installed_games();
    let game = &games[0];
    assert_eq!(game.lua_issues.len(), 1);
    assert!(
        !game.lua_issues[0].active,
        "a commented call loads today: the card must not claim a broken file"
    );
}
