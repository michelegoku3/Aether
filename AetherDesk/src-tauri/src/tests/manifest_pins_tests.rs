//! Regression tests for `apply_build_pins` (the version-switch pipeline).
//!
//! A build's depot list is a PATCH DIFF: it only contains the depots that
//! changed in that build. A depot absent from the list therefore means "this
//! depot did not change in this patch" (e.g. the Windows/Linux/arch variants
//! that often skip a patch) — never "this depot was removed". Auto-apply must
//! leave absent depots untouched instead of disabling them.

use crate::manifest::pins::{
    DepotManifestPin, LuaManifestEdit, LuaManifestPins, ManifestGidProblem,
};
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
    // One directory PER CALL: `cargo test` runs the tests of this file on
    // parallel threads inside a single process, so a process-wide name made
    // every test delete and recreate the directory its neighbours were writing
    // into (flaky "No such file or directory" from `write_lua`).
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let base = std::env::temp_dir().join(format!(
        "aether_manifest_pins_test_{}_{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
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

// ============================================================================
// F4 — malformed setManifestid lines must be visible and repairable
// ============================================================================

/// A Lua whose second pin carries the exact defect seen in the field: the GID
/// is written in hex, so AetherDLL's `setmanifestid` binding raises and the
/// script engine rejects the WHOLE file (every depot of that game loses its
/// override). This is game 3558400's file, reduced to its essentials.
const LUA_WITH_HEX_GID: &str = r#"-- Backseat Drivers
addappid(3558401, 1, "key1")
setManifestid(3558401, "1111111111111111111")
addappid(3558402, 1, "key2")
setManifestid(3558402, 0x1234567890ABCDEF)
addappid(3558403, 1, "key3")
setManifestid(3558403, "3333333333333333333")
"#;

fn write_lua_content(tmp: &TempDirGuard, app_id: u32, content: &str) -> LuaManifestPins {
    let steam_path = tmp.path();
    let stplug = steam_path.join("config").join("stplug-in");
    fs::create_dir_all(&stplug).expect("create stplug-in dir");
    fs::write(stplug.join(format!("{app_id}.lua")), content).expect("write lua");
    LuaManifestPins::new(steam_path, app_id)
}

#[test]
fn strict_read_still_refuses_a_malformed_lua() {
    let tmp = tempdir();
    let editor = write_lua_content(&tmp, 3558400, LUA_WITH_HEX_GID);

    // Writing is still gated: installing such a file must fail loudly.
    let error = editor.rows_from_file().expect_err("strict read must fail");
    assert!(
        error.contains("Lua line 5"),
        "error must point at the offending line, got: {error}"
    );
    assert!(
        error.contains("decimal uint64"),
        "error must state the runtime rule, got: {error}"
    );
}

#[test]
fn editor_read_keeps_valid_rows_and_marks_the_malformed_one() {
    let tmp = tempdir();
    let editor = write_lua_content(&tmp, 3558400, LUA_WITH_HEX_GID);

    let rows = editor
        .editor_rows_from_file()
        .expect("the editor must open a file the strict gate rejects");
    assert_eq!(rows.len(), 3, "two valid pins + one malformed line");

    let invalid: Vec<_> = rows.iter().filter(|row| row.issue.is_some()).collect();
    assert_eq!(invalid.len(), 1);
    let issue = invalid[0].issue.as_ref().expect("issue present");
    assert_eq!(issue.line, 5, "1-based line, as Steam reports it");
    assert_eq!(issue.depot_id, Some(3558402));
    assert_eq!(issue.raw_value, "0x1234567890ABCDEF");
    assert!(
        issue.active,
        "an uncommented call is executed, so the whole file is rejected"
    );
    assert_eq!(
        invalid[0].manifest_id, "0x1234567890ABCDEF",
        "the raw value is carried so the editor can show what has to be replaced"
    );

    // The healthy rows are untouched and still editable.
    let valid: Vec<_> = rows.iter().filter(|row| row.issue.is_none()).collect();
    assert_eq!(valid.len(), 2);
    assert!(valid.iter().all(|row| row.enabled));
}

#[test]
fn apply_edits_repairs_a_malformed_line_and_the_file_becomes_valid() {
    let tmp = tempdir();
    let editor = write_lua_content(&tmp, 3558400, LUA_WITH_HEX_GID);

    let (content, rows) = editor
        .preview_edits(&[LuaManifestEdit {
            row_id: 4,
            manifest_id: Some("9999999999999999999".to_string()),
            enabled: true,
        }])
        .expect("repairing the malformed line must be accepted");

    assert!(
        LuaManifestPins::validate_content(&content).is_ok(),
        "the repaired file must pass the strict write gate"
    );
    assert!(
        content.contains("setManifestid(3558402, \"9999999999999999999\")"),
        "the broken argument list is rewritten, got:\n{content}"
    );
    // The repair preserves the surrounding structure: same number of pins.
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.issue.is_none()));
}

#[test]
fn apply_edits_refuses_a_repair_that_is_not_a_decimal_gid() {
    let tmp = tempdir();
    let editor = write_lua_content(&tmp, 3558400, LUA_WITH_HEX_GID);

    let error = editor
        .preview_edits(&[LuaManifestEdit {
            row_id: 4,
            manifest_id: Some("0x1234".to_string()),
            enabled: true,
        }])
        .expect_err("a non-decimal repair must be refused before writing");
    assert!(error.contains("decimal uint64"), "got: {error}");
}

#[test]
fn apply_edits_can_comment_out_a_malformed_line() {
    let tmp = tempdir();
    let editor = write_lua_content(&tmp, 3558400, LUA_WITH_HEX_GID);

    let (content, _rows) = editor
        .preview_edits(&[LuaManifestEdit {
            row_id: 4,
            manifest_id: None,
            enabled: false,
        }])
        .expect("disabling a malformed line is a valid resolution");

    let commented = content
        .lines()
        .any(|line| line.trim_start().starts_with("--") && line.contains("3558402"));
    assert!(commented, "the malformed call must be commented out:\n{content}");

    // Turning the row off resolves the *runtime* problem: the script engine no
    // longer executes the broken call, so the file loads with all its other
    // pins. The stricter install gate still refuses a commented malformed pin
    // on purpose (it is a trap for the next "enable updates"), which is why
    // the two gates exist separately.
    assert!(
        LuaManifestPins::validate_executable_content(&content).is_ok(),
        "no executable malformed call may remain"
    );
    assert!(
        LuaManifestPins::validate_content(&content).is_err(),
        "the install gate must still flag the commented malformed pin"
    );
    let inactive = LuaManifestPins::editor_rows_from_content(&content)
        .into_iter()
        .find_map(|row| row.issue)
        .expect("the commented line is still reported, as inactive");
    assert!(!inactive.active, "a commented call cannot break the current load");
}

#[test]
fn issue_severity_follows_whether_the_call_is_executed() {
    // Same broken GID, commented: the file loads today (inactive issue) and
    // the strict install gate still refuses it.
    let commented = "--setManifestid(3558402, 0x1234567890ABCDEF)\n";
    assert!(LuaManifestPins::validate_executable_content(commented).is_ok());
    assert!(LuaManifestPins::validate_content(commented).is_err());

    // Active: the whole file is rejected by AetherDLL.
    let active = "setManifestid(3558402, 0x1234567890ABCDEF)\n";
    assert!(LuaManifestPins::validate_executable_content(active).is_err());
}

#[test]
fn apply_edits_still_refuses_a_typed_value_that_is_not_decimal() {
    let tmp = tempdir();
    let editor = write_lua_content(&tmp, 3558400, LUA);

    let error = editor
        .preview_edits(&[LuaManifestEdit {
            row_id: 5,
            manifest_id: Some("not-a-gid".to_string()),
            enabled: true,
        }])
        .expect_err("garbage input must be rejected");
    assert!(error.contains("decimal uint64"), "got: {error}");
}

#[test]
fn updates_are_enabled_still_answers_for_a_file_with_a_malformed_line() {
    let tmp = tempdir();
    let editor = write_lua_content(&tmp, 3558400, LUA_WITH_HEX_GID);

    // The predicate must not turn an unrelated malformed line into a hard
    // error: that is what blocked the update toggle, the manifest repair and
    // the background synchronizer for this game (which then retried forever).
    // This Lua is version-locked (every pin is uncommented), so the honest
    // answer is `false` — what matters is that it ANSWERS.
    let enabled = editor
        .updates_are_enabled()
        .expect("the predicate must still answer");
    assert!(!enabled, "a fully pinned Lua is not updates-enabled");
}

#[test]
fn unquoted_gid_is_reported_as_a_value_problem_not_a_shape_problem() {
    // An unquoted GID is still a value the binding rejects with the very same
    // "must be a decimal uint64" message, so the diagnosis must carry it
    // instead of falling back to "malformed call".
    let editor = LuaManifestPins::new(std::env::temp_dir(), 1);
    let rows = LuaManifestPins::editor_rows_from_content(
        "setManifestid(3558402, 0x1234567890ABCDEF)\n",
    );
    let issue = rows[0].issue.as_ref().expect("one issue");
    assert_eq!(issue.raw_value, "0x1234567890ABCDEF");
    assert!(issue.active);

    let quoted = LuaManifestPins::editor_rows_from_content("setManifestid(3558402, \"abc\")\n");
    assert_eq!(quoted[0].issue.as_ref().expect("one issue").raw_value, "abc");

    // Nothing to repair in place: no argument at all.
    let shapeless = LuaManifestPins::editor_rows_from_content("setManifestid(3558402)\n");
    let issue = shapeless[0].issue.as_ref().expect("one issue");
    assert_eq!(issue.raw_value, "");
    assert_eq!(issue.problem, ManifestGidProblem::MalformedCall);
    assert!(!editor.path_exists());
}

#[test]
fn active_issue_filter_ignores_commented_lines() {
    // The library card asks a narrower question than the editor: "does this
    // game work right now?". A commented bad call is an inactive issue (the
    // editor shows it in amber) and must not turn a working game red.
    let content = "setManifestid(3558402, 0x1234)\n--setManifestid(3558403, 0x5678)\n";
    assert_eq!(
        crate::manifest::pins::invalid_manifest_rows(content).len(),
        2,
        "the editor sees both lines"
    );
    let active = crate::manifest::pins::active_invalid_manifest_rows(content);
    assert_eq!(active.len(), 1, "only the executed call breaks the load");
    assert_eq!(active[0].line, 1);
    assert_eq!(active[0].depot_id, Some(3558402));
}

#[test]
fn quoted_non_decimal_value_is_reported_with_its_text() {
    // The classification must not depend on the value being unquoted: a quoted
    // but non-decimal GID is the most common real-world defect (a hex GID
    // copied from another tool, a truncated paste), and the editor shows the
    // exact text it has to replace.
    let rows = LuaManifestPins::editor_rows_from_content("setManifestid(3558402, \"0xabc\")\n");
    let issue = rows[0].issue.as_ref().expect("one issue");
    assert_eq!(issue.problem, ManifestGidProblem::NotDecimalUint64);
    assert_eq!(issue.raw_value, "0xabc");
    assert!(issue.active);

    // Trailing spaces inside the quotes are part of the offending value: the
    // binding rejects them too, so the message must show them.
    let rows = LuaManifestPins::editor_rows_from_content("setManifestid(3558402, \" 12 34 \")\n");
    let issue = rows[0].issue.as_ref().expect("one issue");
    assert_eq!(issue.raw_value, " 12 34 ");
}
