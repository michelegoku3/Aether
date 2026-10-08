//! In-memory Lua transformations.
//!
//! This module never reads or writes files. Every operation preserves the
//! number of `setManifestid` rows and returns complete content to the facade,
//! which performs compare-and-write persistence.

use super::document;
use super::line_edit::{join_lua_lines, rewrite_setmanifest_line, set_commented};
use super::model::{ApplyBuildResult, DepotManifestPin, LuaManifestEdit, LuaManifestRow};
use super::parser::{ensure_decimal_uint64, invalid_rows, parse_pins};
use std::collections::HashMap;

pub(super) fn apply_build_pins(
    content: &str,
    pins: &[DepotManifestPin],
) -> Result<(String, ApplyBuildResult), String> {
    document::validate_content(content)?;
    let current = parse_pins(content);
    let before_count = current.len();
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let wanted: HashMap<u32, &str> = pins
        .iter()
        .map(|pin| (pin.depot_id, pin.manifest_id.as_str()))
        .collect();
    let mut applied = 0usize;

    for pin in &current {
        match wanted.get(&pin.row.app_id) {
            Some(manifest_id) => {
                if let Some(addappid_line) = pin.addappid_line {
                    set_commented(&mut lines[addappid_line], false);
                }
                set_commented(&mut lines[pin.setmanifest_line], false);
                if *manifest_id != pin.row.manifest_id {
                    crate::desk_log_info!(
                        "manifest",
                        "Depot {}: manifest modified from '{}' to '{}'",
                        pin.row.app_id,
                        pin.row.manifest_id,
                        manifest_id
                    );
                    lines[pin.setmanifest_line] =
                        rewrite_setmanifest_line(&lines[pin.setmanifest_line], manifest_id)?;
                } else {
                    crate::desk_log_info!(
                        "manifest",
                        "Depot {}: manifest unchanged ('{}')",
                        pin.row.app_id,
                        manifest_id
                    );
                }
                applied += 1;
            }
            None => crate::desk_log_info!(
                "manifest",
                "Depot {}: absent from build snapshot, preserved as-is ('{}')",
                pin.row.app_id,
                pin.row.manifest_id
            ),
        }
    }

    let next_content = join_lua_lines(&lines);
    ensure_same_pin_count(before_count, parse_pins(&next_content).len())?;
    Ok((
        next_content,
        ApplyBuildResult {
            applied_pins: applied,
        },
    ))
}

pub(super) fn realign_commented_pins(
    content: &str,
    pins: &[DepotManifestPin],
) -> Result<(Option<String>, usize), String> {
    document::validate_content(content)?;
    let current = parse_pins(content);
    let before_count = current.len();
    let wanted: HashMap<u32, &str> = pins
        .iter()
        .map(|pin| (pin.depot_id, pin.manifest_id.as_str()))
        .collect();
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let mut realigned = 0usize;

    for pin in &current {
        if pin.row.enabled || !pin.addappid_enabled {
            continue;
        }
        let Some(manifest_id) = wanted.get(&pin.row.app_id) else {
            continue;
        };
        if *manifest_id == pin.row.manifest_id {
            continue;
        }
        lines[pin.setmanifest_line] =
            rewrite_setmanifest_line(&lines[pin.setmanifest_line], manifest_id)?;
        realigned += 1;
    }
    if realigned == 0 {
        return Ok((None, 0));
    }

    let next_content = join_lua_lines(&lines);
    ensure_same_pin_count(before_count, parse_pins(&next_content).len())?;
    Ok((Some(next_content), realigned))
}

pub(super) fn set_updates_enabled(
    content: &str,
    enabled: bool,
) -> Result<(Option<String>, usize), String> {
    document::validate_content(content)?;
    let pins = parse_pins(content);
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let mut changed = 0usize;

    for pin in &pins {
        if !pin.addappid_enabled {
            continue;
        }
        let before = lines[pin.setmanifest_line].clone();
        set_commented(&mut lines[pin.setmanifest_line], enabled);
        if lines[pin.setmanifest_line] != before {
            changed += 1;
        }
    }

    let next_content = join_lua_lines(&lines);
    ensure_same_pin_count(pins.len(), parse_pins(&next_content).len())?;
    if changed == 0 {
        Ok((None, 0))
    } else {
        Ok((Some(next_content), changed))
    }
}

pub(super) fn preview_edits(
    content: &str,
    edits: &[LuaManifestEdit],
) -> Result<(String, Vec<LuaManifestRow>), String> {
    let pins = parse_pins(content);
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let issues = invalid_rows(content);
    let before_total = pins.len() + issues.len();

    for edit in edits {
        if let Some(pin) = pins.iter().find(|pin| pin.row.row_id == edit.row_id) {
            if let Some(addappid_line) = pin.addappid_line {
                set_commented(&mut lines[addappid_line], !edit.enabled);
            }
            set_commented(&mut lines[pin.setmanifest_line], !edit.enabled);
            if let Some(next_manifest_id) = edit.manifest_id.as_deref().map(str::trim) {
                if !next_manifest_id.is_empty() && next_manifest_id != pin.row.manifest_id {
                    ensure_decimal_uint64(next_manifest_id)?;
                    crate::desk_log_info!(
                        "manifest",
                        "Depot {}: manifest modified from '{}' to '{}' (manual edit)",
                        pin.row.app_id,
                        pin.row.manifest_id,
                        next_manifest_id
                    );
                    lines[pin.setmanifest_line] =
                        rewrite_setmanifest_line(&lines[pin.setmanifest_line], next_manifest_id)?;
                }
            }
            continue;
        }

        let Some(issue) = issues
            .iter()
            .find(|issue| issue.line.saturating_sub(1) == edit.row_id)
        else {
            return Err(format!("setManifestid row {} was not found", edit.row_id));
        };
        let Some(target) = lines.get_mut(edit.row_id) else {
            return Err(format!("setManifestid row {} was not found", edit.row_id));
        };

        if !edit.enabled {
            set_commented(target, true);
            crate::desk_log_info!(
                "manifest",
                "Lua line {}: malformed setManifestid commented out (depot {:?})",
                issue.line,
                issue.depot_id
            );
            continue;
        }

        let Some(next_manifest_id) = edit.manifest_id.as_deref().map(str::trim) else {
            continue;
        };
        if next_manifest_id.is_empty() {
            continue;
        }
        ensure_decimal_uint64(next_manifest_id)?;
        *target = rewrite_setmanifest_line(target, next_manifest_id)?;
        crate::desk_log_info!(
            "manifest",
            "Lua line {}: malformed setManifestid '{}' repaired to '{}'",
            issue.line,
            issue.raw_value,
            next_manifest_id
        );
    }

    let next_content = join_lua_lines(&lines);
    let after_total = parse_pins(&next_content).len() + invalid_rows(&next_content).len();
    ensure_same_pin_count(before_total, after_total)?;
    document::validate_executable_content(&next_content)?;
    let rows = document::rows_from_content(&next_content);
    Ok((next_content, rows))
}

fn ensure_same_pin_count(before: usize, after: usize) -> Result<(), String> {
    if after != before {
        return Err(format!(
            "Safety check failed: setManifestid count changed from {} to {}. File was not saved.",
            before, after
        ));
    }
    Ok(())
}
