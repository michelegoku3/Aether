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

/// Standard individual SteamID64 universe/type/instance base.
const STEAM_ID64_INDIVIDUAL_BASE: u64 = 76_561_197_960_265_728;

/// Converts a 32-bit Steam account ID (e.g. `ActiveProcess\ActiveUser`) into
/// its decimal SteamID64 string.
pub fn steam_id64_from_account_id(account_id: u32) -> String {
    (STEAM_ID64_INDIVIDUAL_BASE + u64::from(account_id)).to_string()
}

struct ParsedAccount {
    account: SteamAccount,
    auto_login: bool,
    most_recent: bool,
    timestamp: u64,
}

fn parse_account_entries(text: &str) -> Vec<ParsedAccount> {
    let mut entries: Vec<ParsedAccount> = Vec::new();
    let mut depth = 0usize;
    let mut current_id: Option<String> = None;
    let mut account_name = String::new();
    let mut persona_name = String::new();
    let mut auto_login = false;
    let mut most_recent = false;
    let mut timestamp = 0u64;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            depth += 1;
            if depth == 2 {
                account_name.clear();
                persona_name.clear();
                auto_login = false;
                most_recent = false;
                timestamp = 0;
            }
            continue;
        }
        if trimmed.starts_with('}') {
            if depth == 2 {
                if let Some(steam_id) = current_id.take() {
                    entries.push(ParsedAccount {
                        account: SteamAccount {
                            steam_id,
                            account_name: std::mem::take(&mut account_name),
                            persona_name: std::mem::take(&mut persona_name),
                        },
                        auto_login,
                        most_recent,
                        timestamp,
                    });
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
                } else if key.eq_ignore_ascii_case("AutoLogin") {
                    auto_login = value == "1";
                } else if key.eq_ignore_ascii_case("MostRecent") {
                    most_recent = value == "1";
                } else if key.eq_ignore_ascii_case("timestamp") {
                    if let Ok(ts) = value.parse::<u64>() {
                        timestamp = ts;
                    }
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

    entries
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
    let mut accounts: Vec<SteamAccount> = parse_account_entries(text)
        .into_iter()
        .map(|entry| entry.account)
        .filter(|account| !account.persona_name.is_empty() || !account.account_name.is_empty())
        .collect();

    accounts.sort_by(|a, b| {
        a.persona_name
            .to_lowercase()
            .cmp(&b.persona_name.to_lowercase())
            .then_with(|| a.steam_id.cmp(&b.steam_id))
    });
    accounts
}

/// Determines which account in `loginusers.vdf` Steam treats as the current
/// login (`AutoLogin "1"` > `MostRecent "1"` > newest `timestamp`).
pub fn select_active_steam_id_from_vdf(text: &str) -> Option<String> {
    let mut best: Option<ParsedAccount> = None;
    for candidate in parse_account_entries(text) {
        let Ok(numeric) = candidate.account.steam_id.parse::<u64>() else {
            continue;
        };
        if candidate.account.steam_id.len() != 17 || numeric < STEAM_ID64_INDIVIDUAL_BASE {
            continue;
        }
        let better = match &best {
            None => true,
            Some(cur) => {
                (
                    candidate.auto_login,
                    candidate.most_recent,
                    candidate.timestamp,
                ) > (cur.auto_login, cur.most_recent, cur.timestamp)
            }
        };
        if better {
            best = Some(candidate);
        }
    }
    best.map(|entry| entry.account.steam_id)
}

#[cfg(target_os = "windows")]
fn active_steam_id_from_registry() -> Option<String> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam\ActiveProcess")
        .ok()?;
    let account_id: u32 = key.get_value("ActiveUser").ok()?;
    if account_id == 0 {
        return None;
    }
    Some(steam_id64_from_account_id(account_id))
}

#[cfg(not(target_os = "windows"))]
fn active_steam_id_from_registry() -> Option<String> {
    None
}

/// Detects the current signed-in (or configured auto-login) Steam account:
/// live `ActiveUser` registry value first, then `loginusers.vdf`.
pub fn detect_active_steam_id(steam_root: &Path) -> Option<(String, &'static str)> {
    if let Some(id) = active_steam_id_from_registry() {
        return Some((id, "registry"));
    }
    if steam_root.as_os_str().is_empty() {
        return None;
    }
    let text = fs::read_to_string(steam_root.join("config").join("loginusers.vdf")).ok()?;
    select_active_steam_id_from_vdf(&text).map(|id| (id, "loginusers"))
}

/// Drops the currently active Steam account from the selectable owner picker
/// candidates so the active login cannot select itself as a "remote" owner.
pub fn filter_other_accounts(
    accounts: Vec<SteamAccount>,
    active_steam_id: Option<&str>,
) -> Vec<SteamAccount> {
    match active_steam_id {
        Some(active) if !active.is_empty() => accounts
            .into_iter()
            .filter(|account| account.steam_id != active)
            .collect(),
        _ => accounts,
    }
}

/// Non-active accounts for the configured Steam path, paired with the detected
/// active `(steam_id64, source)` for diagnostic logging.
pub fn read_other_configured_accounts(
    steam_path: &str,
) -> (Vec<SteamAccount>, Option<(String, &'static str)>) {
    let root = resolve::normalize_steam_path(steam_path);
    let root_path = Path::new(&root);
    let accounts = read_accounts(root_path);
    let active = detect_active_steam_id(root_path);
    let filtered = filter_other_accounts(accounts, active.as_ref().map(|(id, _)| id.as_str()));
    (filtered, active)
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

    #[test]
    fn selects_active_account_from_autologin_mostrecent_or_timestamp() {
        assert_eq!(
            select_active_steam_id_from_vdf(SAMPLE),
            Some("76561199876393402".to_string())
        );

        let no_autologin = r#""users"
{
	"76561199876393402"
	{
		"AccountName"		"robspont15"
		"PersonaName"		"SyntaxCode15"
		"AutoLogin"		"0"
		"timestamp"		"100"
	}
	"76561199523209859"
	{
		"AccountName"		"michelegoku3"
		"PersonaName"		"michelegoku3"
		"AutoLogin"		"0"
		"timestamp"		"200"
	}
}
"#;
        assert_eq!(
            select_active_steam_id_from_vdf(no_autologin),
            Some("76561199523209859".to_string())
        );

        let most_recent = r#""users"
{
	"76561199876393402"
	{
		"AccountName"		"robspont15"
		"MostRecent"		"1"
		"timestamp"		"100"
	}
	"76561199523209859"
	{
		"AccountName"		"michelegoku3"
		"MostRecent"		"0"
		"timestamp"		"999"
	}
}
"#;
        assert_eq!(
            select_active_steam_id_from_vdf(most_recent),
            Some("76561199876393402".to_string())
        );
        assert_eq!(select_active_steam_id_from_vdf(""), None);
    }

    #[test]
    fn filters_out_active_account_from_picker_candidates() {
        let accounts = parse_accounts(SAMPLE);
        let filtered = filter_other_accounts(accounts.clone(), Some("76561199876393402"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].steam_id, "76561199523209859");

        let unchanged = filter_other_accounts(accounts, None);
        assert_eq!(unchanged.len(), 2);

        assert_eq!(
            steam_id64_from_account_id(1881579066),
            "76561199841844794".to_string()
        );
    }
}
