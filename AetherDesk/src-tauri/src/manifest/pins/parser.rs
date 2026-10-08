//! Line-oriented parser for SteamTools/LumaCore manifest Lua.
//!
//! Lua is never executed or normalized. This module is the single definition
//! of valid rows and malformed `setManifestid` diagnostics.

use super::model::{LuaManifestRow, LuaManifestRowIssue, ManifestGidProblem, ParsedPin};
use once_cell::sync::Lazy;
use regex::Regex;

static SETMANIFEST_ANY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\s*(?:--\s*)?setmanifestid\s*\(").expect("valid regex"));

static SETMANIFEST_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"^\s*(?P<comment>--\s*)?(?i:setmanifestid)\s*\(\s*(?P<appid>\d+)\s*,\s*["'](?P<manifest>[^"']+)["']\s*(?:,[^)]*)?\)"#,
    )
    .expect("valid regex")
});

static ADDAPPID_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(?P<comment>--\s*)?(?i:addappid)\s*\(\s*(?P<appid>\d+)\b")
        .expect("valid regex")
});

static SETMANIFEST_SHAPE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"^\s*(?:--\s*)?(?i:setmanifestid)\s*\(\s*\d+\s*,\s*(?:["'][^"']*["']|[^,)\s][^,)]*)\s*(?:,[^)]*)?\)"#,
    )
    .expect("valid regex")
});

pub(super) fn parse_pins(content: &str) -> Vec<ParsedPin> {
    let lines: Vec<&str> = content.lines().collect();
    let mut pins = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        let Some((app_id, manifest_id, setmanifest_commented)) = parse_setmanifest_line(line)
        else {
            continue;
        };
        let addappid_line = find_nearest_addappid(&lines, line_index, app_id);
        let addappid_enabled = addappid_line
            .and_then(|index| {
                parse_addappid_line(lines[index]).map(|(_, commented)| !commented)
            })
            .unwrap_or(!setmanifest_commented);

        pins.push(ParsedPin {
            row: LuaManifestRow {
                row_id: line_index,
                app_id,
                manifest_id,
                enabled: !setmanifest_commented && addappid_enabled,
                issue: None,
            },
            addappid_line,
            addappid_enabled,
            setmanifest_line: line_index,
        });
    }
    pins
}

pub(super) fn invalid_rows(content: &str) -> Vec<LuaManifestRowIssue> {
    let mut issues = Vec::new();
    for (line_index, line) in content.lines().enumerate() {
        if !SETMANIFEST_ANY_RE.is_match(line) || parse_setmanifest_line(line).is_some() {
            continue;
        }
        let (depot_id, raw_value) = loose_setmanifest_args(line).unwrap_or((None, None));
        let problem = if SETMANIFEST_SHAPE_RE.is_match(line) {
            ManifestGidProblem::NotDecimalUint64
        } else {
            ManifestGidProblem::MalformedCall
        };
        issues.push(LuaManifestRowIssue {
            line: line_index + 1,
            depot_id,
            raw_value: raw_value.unwrap_or_default(),
            problem,
            active: !line.trim_start().starts_with("--"),
        });
    }
    issues
}

pub(super) fn ensure_decimal_uint64(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.parse::<u64>().is_err()
        || !value.chars().all(|character| character.is_ascii_digit())
    {
        return Err(format!(
            "Invalid manifest GID '{}': {}",
            value,
            ManifestGidProblem::NotDecimalUint64.reason()
        ));
    }
    Ok(())
}

fn parse_setmanifest_line(line: &str) -> Option<(u32, String, bool)> {
    let captures = SETMANIFEST_CALL_RE.captures(line)?;
    let app_id = captures.name("appid")?.as_str().parse().ok()?;
    let manifest_id = captures.name("manifest")?.as_str();
    ensure_decimal_uint64(manifest_id).ok()?;
    Some((
        app_id,
        manifest_id.to_string(),
        captures.name("comment").is_some(),
    ))
}

fn parse_addappid_line(line: &str) -> Option<(u32, bool)> {
    let captures = ADDAPPID_CALL_RE.captures(line)?;
    Some((
        captures.name("appid")?.as_str().parse().ok()?,
        captures.name("comment").is_some(),
    ))
}

fn find_nearest_addappid(
    lines: &[&str],
    setmanifest_line: usize,
    app_id: u32,
) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .take(setmanifest_line)
        .rev()
        .take_while(|(_, line)| parse_setmanifest_line(line).is_none())
        .find_map(|(index, line)| {
            let (parsed_app_id, _) = parse_addappid_line(line)?;
            (parsed_app_id == app_id).then_some(index)
        })
}

fn loose_setmanifest_args(line: &str) -> Option<(Option<u32>, Option<String>)> {
    let trimmed = line.trim_start();
    let body = trimmed
        .strip_prefix("--")
        .map(str::trim_start)
        .unwrap_or(trimmed);
    let open = body.find('(')?;
    let close = body[open..].find(')').map(|offset| open + offset)?;
    let args = &body[open + 1..close];
    let mut parts = args.splitn(2, ',');
    let depot_id = parts.next().unwrap_or_default().trim().parse::<u32>().ok();
    let value = parts.next().map(|value| {
        let value = value.trim();
        if let Some(rest) = value.strip_prefix('"') {
            return rest.split('"').next().unwrap_or_default().to_string();
        }
        if let Some(rest) = value.strip_prefix('\'') {
            return rest.split('\'').next().unwrap_or_default().to_string();
        }
        value
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    });
    Some((depot_id, value))
}
