//! Restore-on-startup persistence for depot manifests
//! in `aethercore.toml`:
//!
//! ```toml
//! [manifest_cache]
//! restore_on_startup = true   # default ON
//! ```
//!
//! The DLL reads this key into `Settings::manifestRestoreOnStartup`: when
//! true, every Steam start recopies the `*.manifest` files saved under
//! `AetherData\backup\<app_id>\lua` that are missing from `Steam\depotcache`
//! back into place. Uninstalling a game wipes its manifests, and Steam no
//! longer serves manifests without authentication — the local backup is the
//! only copy left, so the default is ON (missing key => enabled).
//! AetherDesk owns both copies (AetherData + legacy `<Steam>/aethercore`).
//!
//! Line-based editing intentional: the file stays hand-editable, comments and
//! unrelated sections are preserved byte-per-line (same approach as
//! `presence_config` / `ost_config`).

use crate::core::presence_config::{find_key_line, upsert_key_line};

pub const SECTION: &str = "[manifest_cache]";
pub const RESTORE_KEY: &str = "restore_on_startup";

/// The copies of aethercore.toml updated by AetherDesk (same dual-path
/// already used for presence keys in `presence_config`).
pub fn aethercore_toml_paths(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    crate::core::presence_config::aethercore_toml_paths(app)
}

/// Reads `restore_on_startup` from the first copy that declares it.
/// Missing file/key => None (callers default to true = ON).
pub fn read_manifest_restore_enabled(path: &std::path::Path) -> Option<bool> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    let idx = find_key_line(&lines, RESTORE_KEY)?;
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

/// Sets `restore_on_startup = true|false` under `[manifest_cache]`, creating
/// the section when missing. Returns true when the file was modified.
pub fn set_manifest_restore_enabled_in_toml(path: &std::path::Path, enabled: bool) -> bool {
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
    if let Some(i) = find_key_line(&lines, RESTORE_KEY) {
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
            Some(c) => format!("{RESTORE_KEY} = {value}  {c}"),
            None => format!("{RESTORE_KEY} = {value}"),
        };
    } else {
        let hdr = lines
            .iter()
            .position(|l| l.trim_start() == SECTION);
        match hdr {
            Some(h) => {
                let mut insertion = h + 1;
                upsert_key_line(&mut lines, RESTORE_KEY, value, &mut insertion);
            }
            None => {
                if !lines.is_empty() && !lines.last().map(|l| l.trim().is_empty()).unwrap_or(true) {
                    lines.push(String::new());
                }
                lines.push(SECTION.to_string());
                lines.push(format!("{RESTORE_KEY} = {value}"));
            }
        }
    }
    let mut out = lines.join("\n");
    if had_trailing_nl || !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(path, out).is_ok()
}

/// Inserts the canonical `[manifest_cache] restore_on_startup = true` key when
/// missing, without touching an explicit user choice (migration for existing
/// installs). Idempotent, comment-safe.
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
    if find_key_line(&lines, RESTORE_KEY).is_some() {
        return;
    }
    match lines.iter().position(|l| l.trim_start() == SECTION) {
        Some(h) => {
            let mut insertion = h + 1;
            upsert_key_line(&mut lines, RESTORE_KEY, "true", &mut insertion);
        }
        None => {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(SECTION.to_string());
            lines.push(format!("{RESTORE_KEY} = true"));
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
        let f = write_tmp("[manifest_cache]\n");
        assert_eq!(read_manifest_restore_enabled(f.path()), None);
    }

    #[test]
    fn set_creates_manifest_cache_section_when_missing() {
        let f = write_tmp("[log]\nlevel = \"trace\"\n");
        assert!(set_manifest_restore_enabled_in_toml(f.path(), false));
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("[manifest_cache]"));
        assert!(content.contains("restore_on_startup = false"));
        assert_eq!(read_manifest_restore_enabled(f.path()), Some(false));
    }

    #[test]
    fn set_is_idempotent_and_keeps_comments() {
        let f = write_tmp("[manifest_cache]\n# keep me\nrestore_on_startup = true  # user choice\n");
        assert!(!set_manifest_restore_enabled_in_toml(f.path(), true));
        assert!(set_manifest_restore_enabled_in_toml(f.path(), false));
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("# keep me"));
        assert!(content.contains("# user choice"));
        assert_eq!(read_manifest_restore_enabled(f.path()), Some(false));
    }

    #[test]
    fn ensure_defaults_does_not_override_false() {
        let f = write_tmp("[manifest_cache]\nrestore_on_startup = false\n");
        ensure_defaults(f.path());
        assert_eq!(read_manifest_restore_enabled(f.path()), Some(false));
    }

    #[test]
    fn ensure_defaults_inserts_true_when_missing() {
        let f = write_tmp("[log]\nlevel = \"trace\"\n");
        ensure_defaults(f.path());
        assert_eq!(read_manifest_restore_enabled(f.path()), Some(true));
    }
}
