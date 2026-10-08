//! Read-only document views and validation policies.

use super::model::{LuaManifestRow, LuaManifestRowIssue};
use super::parser::{invalid_rows, parse_pins};

const HIDDEN_SYSTEM_DEPOTS: &[u32] = &[
    228981, 228982, 228983, 228984, 228985, 228986, 228987, 228988, 228989, 228990, 229000, 229001,
    229002, 229003, 229004, 229005, 229006, 229007, 229010, 229011, 229012, 229020, 229030, 229031,
    229032, 229033,
];

pub(super) fn rows_from_content(content: &str) -> Vec<LuaManifestRow> {
    let mut rows: Vec<LuaManifestRow> = parse_pins(content)
        .into_iter()
        .map(|pin| pin.row)
        .filter(|row| !HIDDEN_SYSTEM_DEPOTS.contains(&row.app_id))
        .collect();
    rows.sort_by_key(|row| row.app_id);
    rows
}

pub(super) fn rows_for_manifest_sync(content: &str) -> Vec<LuaManifestRow> {
    let mut rows: Vec<LuaManifestRow> = parse_pins(content)
        .into_iter()
        .filter(|pin| pin.addappid_enabled)
        .map(|pin| pin.row)
        .filter(|row| !HIDDEN_SYSTEM_DEPOTS.contains(&row.app_id))
        .collect();
    rows.sort_by_key(|row| row.app_id);
    rows
}

pub(super) fn editor_rows_from_content(content: &str) -> Vec<LuaManifestRow> {
    let mut rows = rows_from_content(content);
    rows.extend(invalid_rows(content).into_iter().map(issue_row));
    rows.sort_by_key(|row| (row.app_id, row.row_id));
    rows
}

pub(super) fn validate_content(content: &str) -> Result<(), String> {
    validate_first(invalid_rows(content).into_iter().next())
}

pub(super) fn validate_executable_content(content: &str) -> Result<(), String> {
    validate_first(active_invalid_rows(content).into_iter().next())
}

pub(super) fn active_invalid_rows(content: &str) -> Vec<LuaManifestRowIssue> {
    invalid_rows(content)
        .into_iter()
        .filter(|issue| issue.active)
        .collect()
}

pub(super) fn first_active_issue(content: &str) -> Option<LuaManifestRowIssue> {
    active_invalid_rows(content).into_iter().next()
}

pub(super) fn updates_are_enabled(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    parse_pins(content)
        .into_iter()
        .filter(|pin| pin.addappid_enabled)
        .any(|pin| {
            lines
                .get(pin.setmanifest_line)
                .map(|line| line.trim_start().starts_with("--"))
                .unwrap_or(false)
        })
}

fn validate_first(issue: Option<LuaManifestRowIssue>) -> Result<(), String> {
    match issue {
        Some(issue) => Err(format!(
            "Invalid setManifestid call at Lua line {}: {}",
            issue.line,
            issue.problem.reason()
        )),
        None => Ok(()),
    }
}

fn issue_row(issue: LuaManifestRowIssue) -> LuaManifestRow {
    LuaManifestRow {
        row_id: issue.line.saturating_sub(1),
        app_id: issue.depot_id.unwrap_or(0),
        manifest_id: issue.raw_value.clone(),
        enabled: false,
        issue: Some(issue),
    }
}
