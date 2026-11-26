//! Minimal read-only reader for Steam's `appcache/appinfo.vdf` (v29).
//!
//! Used to surface per-depot host compatibility hints (`oslist`, `osarch`,
//! `steamdeck`) in the Manual Change Version editor, mirroring the exact
//! eligibility rules AetherDLL applies at download/plan time. Any missing,
//! unknown-version, or truncated file fails open (`None` / empty) so the editor
//! never blocks or mislabels depots on incomplete local cache state.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};

const APPINFO_V29_MAGIC: u32 = 0x0756_4429;
const HEADER_LEN: usize = 16;
const ENTRY_META_LEN: usize = 60;
const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;
const MAX_STRING_TABLE_BYTES: usize = 16 * 1024 * 1024;
const MAX_KV_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DepotHostSkipReason {
    WrongOs,
    WrongArch,
    SteamDeckOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepotHostSkip {
    pub reason: DepotHostSkipReason,
    /// Compact pill label shown next to the depot ID (`32-bit`, `Linux`, `macOS`, `Steam Deck`).
    pub label: String,
    /// Human-readable explanation shown in the unlock confirmation popup and tooltip.
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DepotConfigInfo {
    pub os_list: String,
    pub os_arch: String,
    pub steam_deck: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostTarget {
    pub host_os: &'static str,
    pub host_is_64bit: bool,
    pub host_is_steam_deck: bool,
}

impl HostTarget {
    pub fn current() -> Self {
        let steam_deck = std::env::var("SteamDeck")
            .map(|v| is_truthy_flag(&v))
            .unwrap_or(false);
        Self {
            host_os: if cfg!(target_os = "windows") {
                "windows"
            } else if cfg!(target_os = "macos") {
                "macos"
            } else {
                "linux"
            },
            host_is_64bit: cfg!(target_pointer_width = "64"),
            host_is_steam_deck: steam_deck,
        }
    }
}

pub fn is_truthy_flag(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed == "1" || trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("yes")
}

pub fn os_list_allows_host(os_list: &str, host: &str) -> bool {
    let trimmed_list = os_list.trim();
    if trimmed_list.is_empty() || host.is_empty() {
        return true;
    }
    trimmed_list
        .split(',')
        .map(str::trim)
        .any(|token| !token.is_empty() && token.eq_ignore_ascii_case(host))
}

fn format_os_label(os_list: &str) -> String {
    let tokens: Vec<String> = os_list
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| match token.to_ascii_lowercase().as_str() {
            "macos" => "macOS".to_string(),
            "linux" => "Linux".to_string(),
            "windows" => "Windows".to_string(),
            other => other.to_string(),
        })
        .collect();
    if tokens.is_empty() {
        "Other OS".to_string()
    } else {
        tokens.join(", ")
    }
}

/// Evaluates whether a depot is non-applicable on `host` given the full map of
/// depot configs cached for that app.
pub fn evaluate_depot_skip(
    depot_id: u32,
    configs: &HashMap<u32, DepotConfigInfo>,
    host: HostTarget,
) -> Option<DepotHostSkip> {
    let cfg = configs.get(&depot_id)?;
    if !os_list_allows_host(&cfg.os_list, host.host_os) {
        let label = format_os_label(&cfg.os_list);
        let host_label = format_os_label(host.host_os);
        return Some(DepotHostSkip {
            reason: DepotHostSkipReason::WrongOs,
            label: label.clone(),
            detail: format!(
                "Depot {depot_id} targets {label}, while this PC runs {host_label}. Steam and AetherDLL skip this depot automatically."
            ),
        });
    }
    if !host.host_is_steam_deck && is_truthy_flag(&cfg.steam_deck) {
        return Some(DepotHostSkip {
            reason: DepotHostSkipReason::SteamDeckOnly,
            label: "Steam Deck".to_string(),
            detail: format!(
                "Depot {depot_id} targets Steam Deck hardware. Steam and AetherDLL skip this depot automatically on desktop."
            ),
        });
    }
    let arch = cfg.os_arch.trim();
    if host.host_is_64bit {
        let has_64bit_sibling = configs.values().any(|entry| entry.os_arch.trim() == "64");
        if has_64bit_sibling && arch == "32" {
            return Some(DepotHostSkip {
                reason: DepotHostSkipReason::WrongArch,
                label: "32-bit".to_string(),
                detail: format!(
                    "Depot {depot_id} targets 32-bit Windows, while a 64-bit depot is present for this game. Steam and AetherDLL skip this depot automatically on 64-bit Windows."
                ),
            });
        }
    } else if arch == "64" {
        return Some(DepotHostSkip {
            reason: DepotHostSkipReason::WrongArch,
            label: "64-bit".to_string(),
            detail: format!(
                "Depot {depot_id} targets 64-bit Windows, while this system is 32-bit."
            ),
        });
    }
    None
}

/// Reads `<steam_root>/appcache/appinfo.vdf` and returns `depots/<id>/config`
/// for `target_app_id`. Returns an empty map when the cache is absent, mid-write,
/// unsupported, or does not contain `target_app_id`.
pub fn read_app_depot_configs(
    steam_root: &Path,
    target_app_id: u32,
) -> HashMap<u32, DepotConfigInfo> {
    if target_app_id == 0 {
        return HashMap::new();
    }
    let path = steam_root.join("appcache").join("appinfo.vdf");
    match read_app_depot_configs_from_file(&path, target_app_id) {
        Ok(map) => {
            crate::desk_log_debug!(
                "steam",
                "Read appinfo.vdf for {}: {} depot config(s) found",
                crate::core::logger::format_appid(target_app_id),
                map.len()
            );
            map
        }
        Err(reason) => {
            crate::desk_log_debug!(
                "steam",
                "Skipped appinfo.vdf host hints for {}: {}",
                crate::core::logger::format_appid(target_app_id),
                reason
            );
            HashMap::new()
        }
    }
}

fn read_app_depot_configs_from_file(
    path: &Path,
    target_app_id: u32,
) -> Result<HashMap<u32, DepotConfigInfo>, String> {
    let file = File::open(path).map_err(|e| format!("open failed: {e}"))?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("stat failed: {e}"))?
        .len();
    if file_len < HEADER_LEN as u64 {
        return Err("file smaller than v29 header".to_string());
    }
    let mut reader = BufReader::new(file);

    let mut header = [0u8; HEADER_LEN];
    reader
        .read_exact(&mut header)
        .map_err(|e| format!("header read failed: {e}"))?;
    let magic = u32::from_le_bytes(header[0..4].try_into().map_err(|_| "bad slice")?);
    if magic != APPINFO_V29_MAGIC {
        return Err(format!("unsupported magic 0x{magic:08x}"));
    }
    let string_table_offset =
        u64::from_le_bytes(header[8..16].try_into().map_err(|_| "bad slice")?);
    if string_table_offset < HEADER_LEN as u64 || string_table_offset >= file_len {
        return Err("invalid string table offset".to_string());
    }

    let mut pos = HEADER_LEN as u64;
    let mut target_kv: Option<Vec<u8>> = None;
    let mut entry_head = [0u8; 8];
    while pos + 4 <= string_table_offset {
        let mut app_id_buf = [0u8; 4];
        reader
            .read_exact(&mut app_id_buf)
            .map_err(|e| format!("entry appid read failed: {e}"))?;
        let app_id = u32::from_le_bytes(app_id_buf);
        if app_id == 0 {
            break;
        }
        if pos + 8 > string_table_offset {
            return Err("truncated entry header".to_string());
        }
        reader
            .read_exact(&mut entry_head[4..8])
            .map_err(|e| format!("entry size read failed: {e}"))?;
        let size =
            u32::from_le_bytes(entry_head[4..8].try_into().map_err(|_| "bad slice")?) as usize;
        if !(ENTRY_META_LEN..=MAX_ENTRY_BYTES).contains(&size) {
            return Err(format!("invalid entry size {size} for app {app_id}"));
        }
        let next_pos = pos
            .checked_add(8 + size as u64)
            .ok_or_else(|| "offset overflow".to_string())?;
        if next_pos > string_table_offset {
            return Err("entry extends past string table".to_string());
        }

        if app_id == target_app_id {
            reader
                .seek_relative(ENTRY_META_LEN as i64)
                .map_err(|e| format!("seek metadata failed: {e}"))?;
            let kv_len = size - ENTRY_META_LEN;
            let mut kv_bytes = vec![0u8; kv_len];
            reader
                .read_exact(&mut kv_bytes)
                .map_err(|e| format!("kv read failed: {e}"))?;
            target_kv = Some(kv_bytes);
            break;
        }
        reader
            .seek_relative(size as i64)
            .map_err(|e| format!("seek skip failed: {e}"))?;
        pos = next_pos;
    }

    let Some(kv_bytes) = target_kv else {
        return Ok(HashMap::new());
    };

    let string_table_len = usize::try_from(file_len - string_table_offset)
        .map_err(|_| "string table overflows usize".to_string())?;
    if !(4..=MAX_STRING_TABLE_BYTES).contains(&string_table_len) {
        return Err(format!("invalid string table length {string_table_len}"));
    }
    reader
        .seek(SeekFrom::Start(string_table_offset))
        .map_err(|e| format!("seek string table failed: {e}"))?;
    let mut table_bytes = vec![0u8; string_table_len];
    reader
        .read_exact(&mut table_bytes)
        .map_err(|e| format!("read string table failed: {e}"))?;
    let strings = parse_string_table(&table_bytes)?;

    let mut out = HashMap::new();
    let mut cursor = 0usize;
    let mut path_stack: Vec<&str> = Vec::with_capacity(8);
    walk_kv(
        &kv_bytes,
        &mut cursor,
        &strings,
        0,
        &mut path_stack,
        &mut out,
    )?;
    Ok(out)
}

fn parse_string_table(buf: &[u8]) -> Result<Vec<&str>, String> {
    if buf.len() < 4 {
        return Err("truncated string table".to_string());
    }
    let count = u32::from_le_bytes(buf[0..4].try_into().map_err(|_| "bad slice")?) as usize;
    let mut strings = Vec::with_capacity(count.min(65536));
    let mut pos = 4usize;
    for _ in 0..count {
        let rel_end = buf[pos..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| "unterminated string table entry".to_string())?;
        let s = std::str::from_utf8(&buf[pos..pos + rel_end]).unwrap_or("");
        strings.push(s);
        pos += rel_end + 1;
    }
    Ok(strings)
}

fn walk_kv<'a>(
    buf: &'a [u8],
    cursor: &mut usize,
    strings: &[&'a str],
    depth: usize,
    path: &mut Vec<&'a str>,
    out: &mut HashMap<u32, DepotConfigInfo>,
) -> Result<(), String> {
    if depth > MAX_KV_DEPTH {
        return Err("KV nesting too deep".to_string());
    }
    while *cursor < buf.len() {
        let tag = buf[*cursor];
        *cursor += 1;
        if tag == 0x08 {
            return Ok(());
        }
        if *cursor + 4 > buf.len() {
            return Err("truncated KV key index".to_string());
        }
        let key_idx = u32::from_le_bytes(
            buf[*cursor..*cursor + 4]
                .try_into()
                .map_err(|_| "bad slice")?,
        ) as usize;
        *cursor += 4;
        let key = *strings
            .get(key_idx)
            .ok_or_else(|| format!("out-of-bounds string index {key_idx}"))?;

        match tag {
            0x00 => {
                path.push(key);
                walk_kv(buf, cursor, strings, depth + 1, path, out)?;
                path.pop();
            }
            0x01 => {
                let rel_end = buf[*cursor..]
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or_else(|| "unterminated KV string".to_string())?;
                let val = std::str::from_utf8(&buf[*cursor..*cursor + rel_end]).unwrap_or("");
                *cursor += rel_end + 1;
                record_depot_field(path, key, val, out);
            }
            0x02..=0x06 => {
                if *cursor + 4 > buf.len() {
                    return Err("truncated 32-bit KV value".to_string());
                }
                if tag == 0x02 {
                    let num = i32::from_le_bytes(
                        buf[*cursor..*cursor + 4]
                            .try_into()
                            .map_err(|_| "bad slice")?,
                    );
                    record_depot_field(path, key, &num.to_string(), out);
                }
                *cursor += 4;
            }
            0x07 | 0x0A => {
                if *cursor + 8 > buf.len() {
                    return Err("truncated 64-bit KV value".to_string());
                }
                *cursor += 8;
            }
            other => return Err(format!("unknown KV tag 0x{other:02x}")),
        }
    }
    Ok(())
}

fn record_depot_field(
    path: &[&str],
    key: &str,
    value: &str,
    out: &mut HashMap<u32, DepotConfigInfo>,
) {
    // Expected path: ["appinfo", "depots", "<depot_id>", "config"]
    if path.len() != 4
        || !path[0].eq_ignore_ascii_case("appinfo")
        || !path[1].eq_ignore_ascii_case("depots")
        || !path[3].eq_ignore_ascii_case("config")
    {
        return;
    }
    let Ok(depot_id) = path[2].parse::<u32>() else {
        return;
    };
    let entry = out.entry(depot_id).or_default();
    if key.eq_ignore_ascii_case("oslist") {
        entry.os_list = value.to_string();
    } else if key.eq_ignore_ascii_case("osarch") {
        entry.os_arch = value.to_string();
    } else if key.eq_ignore_ascii_case("steamdeck") {
        entry.steam_deck = value.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn build_v29_appinfo(entries: &[(u32, Vec<u8>)], strings: &[&str]) -> Vec<u8> {
        let mut table = Vec::new();
        table.extend_from_slice(&u32::try_from(strings.len()).expect("len").to_le_bytes());
        for s in strings {
            table.extend_from_slice(s.as_bytes());
            table.push(0);
        }

        let mut body = Vec::new();
        for (app_id, kv) in entries {
            body.extend_from_slice(&app_id.to_le_bytes());
            let size = u32::try_from(ENTRY_META_LEN + kv.len()).expect("size");
            body.extend_from_slice(&size.to_le_bytes());
            body.extend_from_slice(&[0u8; ENTRY_META_LEN]);
            body.extend_from_slice(kv);
        }
        body.extend_from_slice(&0u32.to_le_bytes());

        let str_offset = HEADER_LEN as u64 + body.len() as u64;
        let mut out = Vec::new();
        out.extend_from_slice(&APPINFO_V29_MAGIC.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&str_offset.to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&table);
        out
    }

    fn push_obj(buf: &mut Vec<u8>, key_idx: u32, inner: impl FnOnce(&mut Vec<u8>)) {
        buf.push(0x00);
        buf.extend_from_slice(&key_idx.to_le_bytes());
        inner(buf);
        buf.push(0x08);
    }

    fn push_str(buf: &mut Vec<u8>, key_idx: u32, val: &str) {
        buf.push(0x01);
        buf.extend_from_slice(&key_idx.to_le_bytes());
        buf.extend_from_slice(val.as_bytes());
        buf.push(0);
    }

    #[test]
    fn parses_v29_appinfo_depots_and_evaluates_host_skips() {
        let strings = [
            "appinfo",   // 0
            "depots",    // 1
            "config",    // 2
            "oslist",    // 3
            "osarch",    // 4
            "steamdeck", // 5
            "294101",    // 6
            "294104",    // 7
            "294102",    // 8
            "1373962",   // 9
        ];

        let mut rimworld_kv = Vec::new();
        push_obj(&mut rimworld_kv, 0, |b| {
            push_obj(b, 1, |b| {
                push_obj(b, 6, |b| {
                    push_obj(b, 2, |b| {
                        push_str(b, 3, "windows");
                        push_str(b, 4, "32");
                    });
                });
                push_obj(b, 7, |b| {
                    push_obj(b, 2, |b| {
                        push_str(b, 3, "windows");
                        push_str(b, 4, "64");
                    });
                });
                push_obj(b, 8, |b| {
                    push_obj(b, 2, |b| {
                        push_str(b, 3, "macos");
                    });
                });
                push_obj(b, 9, |b| {
                    push_obj(b, 2, |b| {
                        push_str(b, 3, "windows");
                        push_str(b, 5, "1");
                    });
                });
            });
        });
        rimworld_kv.push(0x08);

        let blob = build_v29_appinfo(&[(480, vec![0x08]), (294100, rimworld_kv)], &strings);
        let dir = tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("appcache")).expect("appcache");
        std::fs::write(dir.path().join("appcache").join("appinfo.vdf"), blob).expect("write");

        let configs = read_app_depot_configs(dir.path(), 294100);
        assert_eq!(configs.len(), 4);

        let win64 = HostTarget {
            host_os: "windows",
            host_is_64bit: true,
            host_is_steam_deck: false,
        };
        assert_eq!(evaluate_depot_skip(294104, &configs, win64), None);
        assert_eq!(
            evaluate_depot_skip(294101, &configs, win64).map(|s| s.reason),
            Some(DepotHostSkipReason::WrongArch)
        );
        assert_eq!(
            evaluate_depot_skip(294102, &configs, win64).map(|s| (s.reason, s.label)),
            Some((DepotHostSkipReason::WrongOs, "macOS".to_string()))
        );
        assert_eq!(
            evaluate_depot_skip(1373962, &configs, win64).map(|s| s.reason),
            Some(DepotHostSkipReason::SteamDeckOnly)
        );
    }

    #[test]
    fn keeps_32bit_depot_when_no_64bit_sibling_exists() {
        let mut configs = HashMap::new();
        configs.insert(
            1001,
            DepotConfigInfo {
                os_list: "windows".to_string(),
                os_arch: "32".to_string(),
                steam_deck: String::new(),
            },
        );
        let win64 = HostTarget {
            host_os: "windows",
            host_is_64bit: true,
            host_is_steam_deck: false,
        };
        assert_eq!(evaluate_depot_skip(1001, &configs, win64), None);
    }

    #[test]
    fn missing_or_unknown_appinfo_fails_open() {
        let dir = tempdir().expect("tempdir");
        assert!(read_app_depot_configs(dir.path(), 294100).is_empty());

        std::fs::create_dir_all(dir.path().join("appcache")).expect("appcache");
        std::fs::write(
            dir.path().join("appcache").join("appinfo.vdf"),
            b"not-valid-appinfo-blob",
        )
        .expect("write");
        assert!(read_app_depot_configs(dir.path(), 294100).is_empty());
    }
}
