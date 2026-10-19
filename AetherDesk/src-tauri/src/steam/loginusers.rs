//! Minimal reader for Steam's `config/loginusers.vdf`.
//!
//! The cosmetic library-owner banner lets the user pick which local account's
//! persona Steam shows in "From the library of ...". This module extracts the
//! three fields the picker needs (SteamID64, account name, persona name) and
//! ignores everything else in the file.
//!
//! A missing or unreadable file yields an empty list (the UI then shows a short
//! notice instead of an error). A malformed file is parsed best-effort: the
//! entries it still yields are returned and the rest is ignored.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::fs;
use std::path::Path;

use crate::steam::resolve;

/// `\t"AccountName"\t\t"robspont15"` -> ("AccountName", "robspont15").
static KEY_VALUE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*"([^"]+)"\s+"([^"]*)""#).expect("static regex"));

/// `\t"76561199876393402"` -> "76561199876393402" (block key, brace on the next line).
static SECTION_KEY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*"([^"]+)"\s*$"#).expect("static regex"));

/// One locally remembered Steam account. Serialized camelCase for IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamAccount {
    pub steam_id: String,
    pub account_name: String,
    pub persona_name: String,
}

/// Reads the accounts remembered in `<steam_root>/config/loginusers.vdf`.
pub fn read_accounts(steam_root: &Path) -> Vec<SteamAccount> {
    if steam_root.as_os_str().is_empty() {
        return Vec::new();
    }
    match fs::read_to_string(steam_root.join("config").join("loginusers.vdf")) {
        Ok(text) => parse_accounts(&text),
        Err(_) => Vec::new(),
    }
}

/// Parses the subset of `loginusers.vdf` the picker needs. Pure and testable.
///
/// Shape:
/// ```text
/// "users"
/// {
///     "76561199876393402"
///     {
///         "AccountName"    "robspont15"
///         "PersonaName"    "SyntaxCode15"
///     }
/// }
/// ```
pub fn parse_accounts(text: &str) -> Vec<SteamAccount> {
    let mut accounts: Vec<SteamAccount> = Vec::new();
    let mut depth = 0usize;
    let mut current_id: Option<String> = None;
    let mut account_name = String::new();
    let mut persona_name = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            depth += 1;
            if depth == 2 {
                account_name.clear();
                persona_name.clear();
            }
            continue;
        }
        if trimmed.starts_with('}') {
            if depth == 2 {
                if let Some(steam_id) = current_id.take() {
                    if !persona_name.is_empty() || !account_name.is_empty() {
                        accounts.push(SteamAccount {
                            steam_id,
                            account_name: std::mem::take(&mut account_name),
                            persona_name: std::mem::take(&mut persona_name),
                        });
                    }
                }
            }
            depth = depth.saturating_sub(1);
            continue;
        }
        if let Some(caps) = KEY_VALUE_RE.captures(line) {
            // Only the account block's own fields; the top-level "users" table
            // and any nested sub-table stay untouched.
            if depth == 2 {
                let key = &caps[1];
                let value = caps[2].to_string();
                if key.eq_ignore_ascii_case("AccountName") {
                    account_name = value;
                } else if key.eq_ignore_ascii_case("PersonaName") {
                    persona_name = value;
                }
            }
            continue;
        }
        if let Some(caps) = SECTION_KEY_RE.captures(line) {
            let key = &caps[1];
            if depth == 1 && !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) {
                current_id = Some(key.to_string());
            }
        }
    }

    accounts.sort_by(|a, b| {
        a.persona_name
            .to_lowercase()
            .cmp(&b.persona_name.to_lowercase())
            .then_with(|| a.steam_id.cmp(&b.steam_id))
    });
    accounts
}

/// Convenience wrapper: accounts for the configured Steam path.
pub fn read_configured_accounts(steam_path: &str) -> Vec<SteamAccount> {
    let root = resolve::normalize_steam_path(steam_path);
    read_accounts(Path::new(&root))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#""users"
{
	"76561199876393402"
	{
		"AccountName"		"robspont15"
		"PersonaName"		"SyntaxCode15"
		"RememberPassword"		"1"
		"AutoLogin"		"1"
	}
	"76561199523209859"
	{
		"AccountName"		"michelegoku3"
		"PersonaName"		"michelegoku3"
		"RememberPassword"		"1"
		"AutoLogin"		"0"
	}
}
"#;

    #[test]
    fn parses_both_accounts_sorted_by_persona() {
        let accounts = parse_accounts(SAMPLE);
        assert_eq!(accounts.len(), 2);
        // "michelegoku3" sorts before "SyntaxCode15" (case-insensitive).
        assert_eq!(accounts[0].steam_id, "76561199523209859");
        assert_eq!(accounts[0].account_name, "michelegoku3");
        assert_eq!(accounts[0].persona_name, "michelegoku3");
        assert_eq!(accounts[1].steam_id, "76561199876393402");
        assert_eq!(accounts[1].account_name, "robspont15");
        assert_eq!(accounts[1].persona_name, "SyntaxCode15");
    }

    #[test]
    fn ignores_non_numeric_section_keys_and_other_tables() {
        let text = r#""users"
{
	"unsupported"
	{
		"AccountName"		"nobody"
	}
	"76561199523209859"
	{
		"AccountName"		"michelegoku3"
		"PersonaName"		"michelegoku3"
	}
}
"#;
        let accounts = parse_accounts(text);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].steam_id, "76561199523209859");
    }

    #[test]
    fn malformed_or_empty_input_yields_no_accounts() {
        assert!(parse_accounts("").is_empty());
        assert!(parse_accounts("not a vdf").is_empty());
        assert!(read_accounts(Path::new("")).is_empty());
    }
}
