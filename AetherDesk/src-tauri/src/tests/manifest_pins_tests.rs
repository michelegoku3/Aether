//! Regression tests for `apply_build_pins` (the version-switch pipeline).
//!
//! A build's depot list is a PATCH DIFF: it only contains the depots that
//! changed in that build. A depot absent from the list therefore means "this
//! depot did not change in this patch" (e.g. the Windows/Linux/arch variants
//! that often skip a patch) — never "this depot was removed". Auto-apply must
//! leave absent depots untouched instead of disabling them.

use crate::manifest::pins::{DepotManifestPin, LuaManifestPins};
use std::fs;
use std::path::{Path, PathBuf};

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
    let base = std::env::temp_dir().join(format!(
        "aether_manifest_pins_test_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create temp dir");
    TempDirGuard(base)
}

/// Three depots: Windows, Linux and a shared depot. All pinned and enabled.
const LUA: &str = r#"-- MAIN APPLICATION
addappid(3618390, 1, "basekey") -- Game

-- MAIN APP DEPOTS
addappid(3618391, 1, "key1") -- Windows
setManifestid(3618391, "1111111111111111111")
addappid(3618392, 1, "key2") -- Linux
setManifestid(3618392, "2222222222222222222")
-- SHARED DEPOTS (from other apps)
addappid(3618393, 1, "key3") -- Shared (Shared from App 999)
setManifestid(3618393, "3333333333333333333")
"#;

fn write_lua(tmp: &TempDirGuard, app_id: u32) -> LuaManifestPins {
    let steam_path = tmp.path();
    let stplug = steam_path.join("config").join("stplug-in");
    fs::create_dir_all(&stplug).expect("create stplug-in dir");
    fs::write(stplug.join(format!("{app_id}.lua")), LUA).expect("write lua");
    LuaManifestPins::new(steam_path, app_id)
}

#[test]
fn apply_build_pins_leaves_absent_depots_untouched() {
    let tmp = tempdir();
    let lua = write_lua(&tmp, 3618390);

    // This build's diff only changed the Windows depot; Linux and the shared
    // depot did not receive an update in that patch and are NOT in the list.
    let pins = vec![DepotManifestPin {
        depot_id: 3618391,
        manifest_id: "9999999999999999999".to_string(),
    }];

    let result = lua.apply_build_pins(&pins).expect("apply pins");
    assert_eq!(result.applied_pins, 1);

    let after = fs::read_to_string(lua.lua_path()).expect("read lua");

    // The changed depot got its new manifest and stays active.
    assert!(after.contains("setManifestid(3618391, \"9999999999999999999\")"));
    assert!(after.contains("addappid(3618391, 1, \"key1\") -- Windows"));

    // Unchanged depots keep their pins verbatim and stay enabled.
    assert!(after.contains("setManifestid(3618392, \"2222222222222222222\")"));
    assert!(after.contains("addappid(3618392, 1, \"key2\") -- Linux"));
    assert!(after.contains("setManifestid(3618393, \"3333333333333333333\")"));
    assert!(after.contains("addappid(3618393, 1, \"key3\") -- Shared (Shared from App 999)"));

    // Depots were never commented out.
    assert!(!after.contains("-- setManifestid(3618392"));
    assert!(!after.contains("-- setManifestid(3618393"));
}

#[test]
fn apply_build_pins_reenables_previously_disabled_depot_when_it_changes() {
    let tmp = tempdir();
    let lua = write_lua(&tmp, 3618390);

    // First apply: only Linux changes.
    let pins1 = vec![DepotManifestPin {
        depot_id: 3618392,
        manifest_id: "4444444444444444444".to_string(),
    }];
    let r1 = lua.apply_build_pins(&pins1).expect("apply pins 1");
    assert_eq!(r1.applied_pins, 1);

    // Second apply: only Windows changes — Linux from before is untouched,
    // Windows gets the new manifest.
    let pins2 = vec![DepotManifestPin {
        depot_id: 3618391,
        manifest_id: "5555555555555555555".to_string(),
    }];
    let r2 = lua.apply_build_pins(&pins2).expect("apply pins 2");
    assert_eq!(r2.applied_pins, 1);

    let after = fs::read_to_string(lua.lua_path()).expect("read lua");
    assert!(after.contains("setManifestid(3618391, \"5555555555555555555\")"));
    assert!(after.contains("setManifestid(3618392, \"4444444444444444444\")"));
    assert!(after.contains("setManifestid(3618393, \"3333333333333333333\")"));
}
