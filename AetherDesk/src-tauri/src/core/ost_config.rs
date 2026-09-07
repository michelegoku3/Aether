//! Opt-in persistence for the OST pattern source (`OpenSteam001/steam-monitor`)
//! in `aethercore.toml`:
//!
//! ```toml
//! [network]
//! use_ost_source = false   # opt-in, default OFF
//! ```
//!
//! The DLL reads this key into `Settings::patternUseOstSource` (hot-reloaded
//! on every game launch): when false, `PatternDownloader::BuildPlan` and
//! `PatternEngine::BestExpectedSource` skip the `opensteamtool` source.
//! AetherDesk owns both copies (AetherData + legacy `<Steam>/aethercore`).
//!
//! Line-based editing intentional: the file stays hand-editable, comments and
//! unrelated sections are preserved byte-per-line (same approach as
//! `presence_config`).

use crate::core::presence_config::{find_key_line, upsert_key_line};

pub const OST_KEY: &str = "use_ost_source";

/// The copies of aethercore.toml updated by AetherDesk (same dual-path
/// already used for presence keys in `presence_config`).
pub fn aethercore_toml_paths(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    crate::core::presence_config::aethercore_toml_paths(app)
}

/// Reads `use_ost_source` from the first copy that declares it.
/// Missing file/key => None (callers default to false = opt-in OFF).
pub fn read_ost_enabled(path: &std::path::Path) -> Option<bool> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    let idx = find_key_line(&lines, OST_KEY)?;
    let after_eq = lines[idx].splitn(2, '=').nth(1)?;
    let v = after_eq.trim();
    // Accept `true`/`false` with optional trailing comment.
    let token = v.split([' ', '\t', '#']).next().unwrap_or("").trim();
    match token {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Sets `use_ost_source = true|false` under `[network]`, creating the section
/// when missing. Returns true when the file was modified.
pub fn set_ost_enabled_in_toml(path: &std::path::Path, enabled: bool) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let had_trailing_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if lines.len() == 1 && lines[0].is_empty() {
        lines.clear();
    }
    let value = if enabled { "true" } else { "false" };
    if let Some(i) = find_key_line(&lines, OST_KEY) {
        let cur = lines[i].splitn(2, '=').nth(1).map(|v| {
            v.trim()
                .split([' ', '\t', '#'])
                .next()
                .unwrap_or("")
                .trim()
        });
        if cur == Some(value) {
            return false; // already in the requested state
        }
        // Preserve any trailing comment on the line.
        let comment = lines[i].splitn(2, '=').nth(1).and_then(|v| {
            let hash = v.find('#')?;
            Some(v[hash..].to_string())
        });
        lines[i] = match comment {
            Some(c) => format!("{OST_KEY} = {value}  {c}"),
            None => format!("{OST_KEY} = {value}"),
        };
    } else {
        let hdr = lines
            .iter()
            .position(|l| l.trim_start() == "[network]");
        match hdr {
            Some(h) => {
                let mut insertion = h + 1;
                upsert_key_line(&mut lines, OST_KEY, value, &mut insertion);
            }
            None => {
                if !lines.is_empty() && !lines.last().map(|l| l.trim().is_empty()).unwrap_or(true) {
                    lines.push(String::new());
                }
                lines.push("[network]".to_string());
                lines.push(format!("{OST_KEY} = {value}"));
            }
        }
    }
    let mut out = lines.join("\n");
    if had_trailing_nl || !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(path, out).is_ok()
}

/// Inserts the canonical `[network] use_ost_source = false` key when missing,
/// without touching an explicit user choice (migration for existing installs).
/// Idempotent, comment-safe.
pub fn ensure_defaults(path: &std::path::Path) {
    if !path.exists() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let had_trailing_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if lines.len() == 1 && lines[0].is_empty() {
        lines.clear();
    }
    if find_key_line(&lines, OST_KEY).is_some() {
        return;
    }
    match lines.iter().position(|l| l.trim_start() == "[network]") {
        Some(h) => {
            let mut insertion = h + 1;
            upsert_key_line(&mut lines, OST_KEY, "false", &mut insertion);
        }
        None => {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push("[network]".to_string());
            lines.push(format!("{OST_KEY} = false"));
        }
    }
    let mut out = lines.join("\n");
    if had_trailing_nl || !out.ends_with('\n') {
        out.push('\n');
    }
    let _ = std::fs::write(path, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("tmp file");
        f.write_all(content.as_bytes()).expect("write tmp");
        f
    }

    #[test]
    fn read_missing_key_returns_none() {
        let f = write_tmp("[network]\npattern_mirror = \"\"\n");
        assert_eq!(read_ost_enabled(f.path()), None);
    }

    #[test]
    fn set_creates_network_section_when_missing() {
        let f = write_tmp("[log]\nlevel = \"trace\"\n");
        assert!(set_ost_enabled_in_toml(f.path(), true));
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("[network]"));
        assert!(content.contains("use_ost_source = true"));
        assert_eq!(read_ost_enabled(f.path()), Some(true));
    }

    #[test]
    fn set_is_idempotent_and_keeps_comments() {
        let f = write_tmp("[network]\n# keep me\nuse_ost_source = false  # user choice\n");
        assert!(!set_ost_enabled_in_toml(f.path(), false));
        assert!(set_ost_enabled_in_toml(f.path(), true));
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("# keep me"));
        assert!(content.contains("# user choice"));
        assert_eq!(read_ost_enabled(f.path()), Some(true));
    }

    #[test]
    fn ensure_defaults_does_not_override_true() {
        let f = write_tmp("[network]\nuse_ost_source = true\n");
        ensure_defaults(f.path());
        assert_eq!(read_ost_enabled(f.path()), Some(true));
    }

    #[test]
    fn ensure_defaults_inserts_false_when_missing() {
        let f = write_tmp("[network]\npattern_mirror = \"\"\n");
        ensure_defaults(f.path());
        assert_eq!(read_ost_enabled(f.path()), Some(false));
    }
}
