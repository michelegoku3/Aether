//! Parser for Steam's binary `UserGameStatsSchema_<appid>.bin` cache.
//!
//! Reads from `<steam>/appcache/stats/UserGameStatsSchema_<appid>.bin` first
//! and falls back to `<AetherData>/backup/<appid>/achievements/UserGameStatsSchema_<appid>.bin`.

use crate::achievements::kv1::{parse_binary, KvDict, KvValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SchemaSource {
    SteamCache,
    AetherBackup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressStatSchema {
    pub stat_id: u32,
    pub min_val: f32,
    pub max_val: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaAchievement {
    pub id: u32,
    pub bucket: u32,
    pub bit: u32,
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub hidden: bool,
    pub icon: String,
    pub icon_gray: String,
    pub progress_stat: Option<ProgressStatSchema>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSchema {
    pub app_id: u32,
    pub version: i32,
    pub game_name: Option<String>,
    pub source: SchemaSource,
    pub source_path: PathBuf,
    pub achievement_buckets: BTreeSet<u32>,
    pub stat_buckets: BTreeMap<String, u32>,
    pub achievements: Vec<SchemaAchievement>,
}

pub fn steam_schema_path(steam_root: &Path, app_id: u32) -> PathBuf {
    steam_root
        .join("appcache")
        .join("stats")
        .join(format!("UserGameStatsSchema_{app_id}.bin"))
}

pub fn backup_schema_path(data_root: &Path, app_id: u32) -> PathBuf {
    data_root
        .join("backup")
        .join(app_id.to_string())
        .join("achievements")
        .join(format!("UserGameStatsSchema_{app_id}.bin"))
}

fn value_as_str(val: Option<&KvValue>) -> Option<&str> {
    match val {
        Some(KvValue::Str(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn value_as_i32(val: Option<&KvValue>) -> Option<i32> {
    match val {
        Some(KvValue::Int32(n)) => Some(*n),
        Some(KvValue::Str(s)) => s.trim().parse::<i32>().ok(),
        _ => None,
    }
}

fn value_as_f32(val: Option<&KvValue>) -> Option<f32> {
    match val {
        Some(KvValue::Float32(f)) => Some(*f),
        Some(KvValue::Int32(n)) => Some(*n as f32),
        Some(KvValue::Str(s)) => s.trim().parse::<f32>().ok(),
        _ => None,
    }
}

fn value_as_bool(val: Option<&KvValue>) -> bool {
    match val {
        Some(KvValue::Int32(n)) => *n != 0,
        Some(KvValue::Str(s)) => {
            let trimmed = s.trim();
            trimmed == "1" || trimmed.eq_ignore_ascii_case("true")
        }
        _ => false,
    }
}

fn extract_localized_text(val: Option<&KvValue>) -> String {
    match val {
        Some(KvValue::Str(s)) => s.clone(),
        Some(KvValue::Dict(dict)) => {
            if let Some(eng) = value_as_str(dict.get_ci("english")) {
                if !eng.trim().is_empty() {
                    return eng.to_string();
                }
            }
            for (k, v) in &dict.entries {
                if k.eq_ignore_ascii_case("token") {
                    continue;
                }
                if let KvValue::Str(s) = v {
                    if !s.trim().is_empty() {
                        return s.clone();
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

fn resolve_app_root(root: &KvDict, app_id: u32) -> Option<&KvDict> {
    let app_key = app_id.to_string();
    if let Some(dict) = root.get_dict(&app_key) {
        return Some(dict);
    }
    if root.get_dict("stats").is_some() {
        return Some(root);
    }
    root.entries.iter().find_map(|(_, v)| match v {
        KvValue::Dict(dict) if dict.get_dict("stats").is_some() => Some(dict),
        _ => None,
    })
}

fn parse_progress_stat(
    bit_dict: &KvDict,
    stat_buckets: &BTreeMap<String, u32>,
) -> Option<ProgressStatSchema> {
    let progress = bit_dict.get_dict("progress")?;
    let val_dict = progress.get_dict("value")?;
    let op = value_as_str(val_dict.get_ci("operation"))?;
    if !op.eq_ignore_ascii_case("statvalue") {
        return None;
    }
    let operand1 = value_as_str(val_dict.get_ci("operand1"))?;
    let stat_id = *stat_buckets.get(operand1)?;
    let min_val = value_as_f32(progress.get_ci("min_val")).unwrap_or(0.0);
    let max_val = value_as_f32(progress.get_ci("max_val")).unwrap_or(0.0);
    Some(ProgressStatSchema {
        stat_id,
        min_val,
        max_val,
    })
}

pub fn parse_schema_bytes(
    app_id: u32,
    bytes: &[u8],
    source: SchemaSource,
    source_path: PathBuf,
) -> Result<ParsedSchema, String> {
    let doc = parse_binary(bytes).map_err(|e| {
        format!(
            "Failed to parse achievement schema {}: {e}",
            source_path.display()
        )
    })?;
    let app_dict = resolve_app_root(&doc.root, app_id).ok_or_else(|| {
        format!(
            "Achievement schema {} has no root entry for AppID {app_id}",
            source_path.display()
        )
    })?;

    let version = value_as_i32(app_dict.get_ci("version")).unwrap_or(0);
    let game_name = value_as_str(app_dict.get_ci("gamename"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let mut stat_buckets = BTreeMap::new();
    let mut achievement_buckets = BTreeSet::new();
    let mut achievements = Vec::new();

    if let Some(stats_dict) = app_dict.get_dict("stats") {
        // First pass: register stat bucket names so `progress.value.operand1` resolves regardless of order.
        for (bucket_key, bucket_val) in &stats_dict.entries {
            let Ok(bucket_id) = bucket_key.parse::<u32>() else {
                continue;
            };
            let KvValue::Dict(bucket_dict) = bucket_val else {
                continue;
            };
            if let Some(stat_name) = value_as_str(bucket_dict.get_ci("name")) {
                if !stat_name.is_empty() {
                    stat_buckets.insert(stat_name.to_string(), bucket_id);
                }
            }
        }

        // Second pass: collect all achievement buckets and their bit definitions.
        for (bucket_key, bucket_val) in &stats_dict.entries {
            let Ok(bucket_id) = bucket_key.parse::<u32>() else {
                continue;
            };
            let KvValue::Dict(bucket_dict) = bucket_val else {
                continue;
            };
            let Some(bits_dict) = bucket_dict.get_dict("bits") else {
                continue;
            };
            achievement_buckets.insert(bucket_id);

            for (bit_key, bit_val) in &bits_dict.entries {
                let Ok(bit) = bit_key.parse::<u32>() else {
                    continue;
                };
                if bit >= 32 {
                    continue;
                }
                let KvValue::Dict(bit_dict) = bit_val else {
                    continue;
                };
                let api_name = value_as_str(bit_dict.get_ci("name"))
                    .unwrap_or("")
                    .to_string();
                let display = bit_dict.get_dict("display");
                let mut name = display
                    .map(|d| extract_localized_text(d.get_ci("name")))
                    .unwrap_or_default();
                if name.is_empty() {
                    name = api_name.clone();
                }
                let description = display
                    .map(|d| extract_localized_text(d.get_ci("desc")))
                    .unwrap_or_default();
                let hidden = display
                    .map(|d| value_as_bool(d.get_ci("hidden")))
                    .unwrap_or(false);
                let icon = display
                    .and_then(|d| value_as_str(d.get_ci("icon")))
                    .unwrap_or("")
                    .to_string();
                let icon_gray = display
                    .and_then(|d| value_as_str(d.get_ci("icon_gray")))
                    .unwrap_or("")
                    .to_string();
                let progress_stat = parse_progress_stat(bit_dict, &stat_buckets);

                achievements.push(SchemaAchievement {
                    id: bucket_id * 32 + bit,
                    bucket: bucket_id,
                    bit,
                    api_name,
                    name,
                    description,
                    hidden,
                    icon,
                    icon_gray,
                    progress_stat,
                });
            }
        }
    }

    achievements.sort_by_key(|a| (a.bucket, a.bit));

    Ok(ParsedSchema {
        app_id,
        version,
        game_name,
        source,
        source_path,
        achievement_buckets,
        stat_buckets,
        achievements,
    })
}

pub fn load_schema_for_app(
    steam_root: &Path,
    data_root: &Path,
    app_id: u32,
) -> Result<ParsedSchema, String> {
    let steam_path = steam_schema_path(steam_root, app_id);
    let backup_path = backup_schema_path(data_root, app_id);
    let candidates = [
        (SchemaSource::SteamCache, steam_path.as_path()),
        (SchemaSource::AetherBackup, backup_path.as_path()),
    ];

    let mut errors = Vec::new();
    for (source, path) in candidates {
        match fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => {
                match parse_schema_bytes(app_id, &bytes, source, path.to_path_buf()) {
                    Ok(parsed) => {
                        let hidden_count = parsed.achievements.iter().filter(|a| a.hidden).count();
                        let counter_count = parsed
                            .achievements
                            .iter()
                            .filter(|a| a.progress_stat.is_some())
                            .count();
                        crate::desk_log_info!(
                            "achievements",
                            "Schema loaded app_id={} source={:?} path={} version={} count={} buckets={} hidden={} counters={}",
                            app_id,
                            source,
                            path.display(),
                            parsed.version,
                            parsed.achievements.len(),
                            parsed.achievement_buckets.len(),
                            hidden_count,
                            counter_count
                        );
                        return Ok(parsed);
                    }
                    Err(err) => {
                        crate::desk_log_warn!(
                            "achievements",
                            "Schema unreadable app_id={} source={:?} path={}: {}",
                            app_id,
                            source,
                            path.display(),
                            err
                        );
                        errors.push(err);
                    }
                }
            }
            Ok(_) => {
                crate::desk_log_debug!(
                    "achievements",
                    "Schema file empty app_id={} source={:?} path={}",
                    app_id,
                    source,
                    path.display()
                );
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                crate::desk_log_debug!(
                    "achievements",
                    "Schema not present app_id={} source={:?} path={}",
                    app_id,
                    source,
                    path.display()
                );
            }
            Err(err) => {
                let msg = format!("Cannot read schema {}: {err}", path.display());
                crate::desk_log_warn!("achievements", "{msg}");
                errors.push(msg);
            }
        }
    }

    if let Some(first_err) = errors.into_iter().next() {
        return Err(first_err);
    }

    Err(format!(
        "NO_SCHEMA: No achievement schema cache found for AppID {app_id}. Open this game's Achievements page in Steam (with AetherDLL active) once so Steam caches its schema if the game has achievements."
    ))
}

pub fn resolve_icon_url(app_id: u32, icon_value: &str) -> Option<String> {
    let trimmed = icon_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    let clean = trimmed.trim_start_matches('/');
    if clean.contains('/') {
        return Some(format!(
            "https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/{clean}"
        ));
    }
    Some(format!(
        "https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/{app_id}/{clean}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::achievements::kv1::{serialize_binary, KvDict, KvDocument, KvValue};
    use tempfile::TempDir;

    fn kv_dict(entries: Vec<(&str, KvValue)>) -> KvValue {
        KvValue::Dict(KvDict {
            entries: entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        })
    }

    fn build_schema_doc(app_id: u32) -> KvDocument {
        KvDocument {
            root: KvDict {
                entries: vec![(
                    app_id.to_string(),
                    kv_dict(vec![
                        (
                            "stats",
                            kv_dict(vec![
                                (
                                    "0",
                                    kv_dict(vec![
                                        ("type", KvValue::Str("ACHIEVEMENTS".to_string())),
                                        (
                                            "bits",
                                            kv_dict(vec![
                                                (
                                                    "0",
                                                    kv_dict(vec![
                                                        (
                                                            "name",
                                                            KvValue::Str("ACH_FIRST".to_string()),
                                                        ),
                                                        (
                                                            "display",
                                                            kv_dict(vec![
                                                                (
                                                                    "name",
                                                                    kv_dict(vec![
                                                                        (
                                                                            "english",
                                                                            KvValue::Str(
                                                                                "First Step"
                                                                                    .to_string(),
                                                                            ),
                                                                        ),
                                                                        (
                                                                            "token",
                                                                            KvValue::Str(
                                                                                "NEW_ACH_0_0_NAME"
                                                                                    .to_string(),
                                                                            ),
                                                                        ),
                                                                    ]),
                                                                ),
                                                                (
                                                                    "desc",
                                                                    kv_dict(vec![(
                                                                        "english",
                                                                        KvValue::Str(
                                                                            "Take the first step"
                                                                                .to_string(),
                                                                        ),
                                                                    )]),
                                                                ),
                                                                ("hidden", KvValue::Int32(0)),
                                                                (
                                                                    "icon",
                                                                    KvValue::Str(
                                                                        "abc.jpg".to_string(),
                                                                    ),
                                                                ),
                                                                (
                                                                    "icon_gray",
                                                                    KvValue::Str(
                                                                        "abc_gray.jpg".to_string(),
                                                                    ),
                                                                ),
                                                            ]),
                                                        ),
                                                    ]),
                                                ),
                                                (
                                                    "4",
                                                    kv_dict(vec![
                                                        (
                                                            "name",
                                                            KvValue::Str(
                                                                "ACH_COLLECTOR".to_string(),
                                                            ),
                                                        ),
                                                        (
                                                            "display",
                                                            kv_dict(vec![
                                                                (
                                                                    "name",
                                                                    kv_dict(vec![(
                                                                        "english",
                                                                        KvValue::Str(
                                                                            "Collector".to_string(),
                                                                        ),
                                                                    )]),
                                                                ),
                                                                (
                                                                    "desc",
                                                                    kv_dict(vec![(
                                                                        "english",
                                                                        KvValue::Str(
                                                                            "Find 10 items"
                                                                                .to_string(),
                                                                        ),
                                                                    )]),
                                                                ),
                                                                // Older Valve schemas encode hidden as string "1"
                                                                (
                                                                    "hidden",
                                                                    KvValue::Str("1".to_string()),
                                                                ),
                                                                (
                                                                    "icon",
                                                                    KvValue::Str(
                                                                        "col.jpg".to_string(),
                                                                    ),
                                                                ),
                                                                (
                                                                    "icon_gray",
                                                                    KvValue::Str(
                                                                        "col_gray.jpg".to_string(),
                                                                    ),
                                                                ),
                                                            ]),
                                                        ),
                                                        (
                                                            "progress",
                                                            kv_dict(vec![
                                                                (
                                                                    "min_val",
                                                                    KvValue::Str("0".to_string()),
                                                                ),
                                                                (
                                                                    "max_val",
                                                                    KvValue::Str("10".to_string()),
                                                                ),
                                                                (
                                                                    "value",
                                                                    kv_dict(vec![
                                                                        (
                                                                            "operation",
                                                                            KvValue::Str(
                                                                                "statvalue"
                                                                                    .to_string(),
                                                                            ),
                                                                        ),
                                                                        (
                                                                            "operand1",
                                                                            KvValue::Str(
                                                                                "ITEMS_FOUND"
                                                                                    .to_string(),
                                                                            ),
                                                                        ),
                                                                    ]),
                                                                ),
                                                            ]),
                                                        ),
                                                    ]),
                                                ),
                                            ]),
                                        ),
                                    ]),
                                ),
                                (
                                    "200",
                                    kv_dict(vec![
                                        ("type", KvValue::Str("ACHIEVEMENTS".to_string())),
                                        (
                                            "bits",
                                            kv_dict(vec![(
                                                "2",
                                                kv_dict(vec![
                                                    ("name", KvValue::Str("ACH_DLC".to_string())),
                                                    (
                                                        "display",
                                                        kv_dict(vec![(
                                                            "name",
                                                            kv_dict(vec![(
                                                                "english",
                                                                KvValue::Str(
                                                                    "High Bucket".to_string(),
                                                                ),
                                                            )]),
                                                        )]),
                                                    ),
                                                ]),
                                            )]),
                                        ),
                                    ]),
                                ),
                                (
                                    "9",
                                    kv_dict(vec![
                                        ("name", KvValue::Str("ITEMS_FOUND".to_string())),
                                        ("type", KvValue::Str("INT".to_string())),
                                    ]),
                                ),
                            ]),
                        ),
                        ("version", KvValue::Int32(7)),
                        ("gamename", KvValue::Str("Test Game".to_string())),
                    ]),
                )],
            },
            has_outer_end: true,
        }
    }

    #[test]
    fn parses_multi_bucket_schema_hidden_variants_and_progress_stat() {
        let bytes = serialize_binary(&build_schema_doc(2124490));
        let parsed = parse_schema_bytes(
            2124490,
            &bytes,
            SchemaSource::SteamCache,
            PathBuf::from("UserGameStatsSchema_2124490.bin"),
        )
        .expect("schema must parse");

        assert_eq!(parsed.app_id, 2124490);
        assert_eq!(parsed.version, 7);
        assert_eq!(parsed.game_name.as_deref(), Some("Test Game"));
        assert_eq!(parsed.achievement_buckets, BTreeSet::from([0, 200]));
        assert_eq!(parsed.stat_buckets.get("ITEMS_FOUND"), Some(&9));
        assert_eq!(parsed.achievements.len(), 3);

        let a0 = &parsed.achievements[0];
        assert_eq!(a0.id, 0);
        assert_eq!(a0.name, "First Step");
        assert!(!a0.hidden);
        assert_eq!(a0.progress_stat, None);

        let a1 = &parsed.achievements[1];
        assert_eq!(a1.id, 4);
        assert!(a1.hidden, "string \"1\" must parse as hidden=true");
        assert_eq!(
            a1.progress_stat,
            Some(ProgressStatSchema {
                stat_id: 9,
                min_val: 0.0,
                max_val: 10.0,
            })
        );

        let a2 = &parsed.achievements[2];
        assert_eq!(a2.id, 200 * 32 + 2);
        assert_eq!(a2.bucket, 200);
        assert_eq!(a2.bit, 2);
    }

    #[test]
    fn loads_from_backup_when_steam_cache_schema_is_missing() {
        let tmp = TempDir::new().unwrap();
        let steam = tmp.path().join("steam");
        let data = tmp.path().join("AetherData");
        let backup_schema = data
            .join("backup")
            .join("2124490")
            .join("achievements")
            .join("UserGameStatsSchema_2124490.bin");
        std::fs::create_dir_all(backup_schema.parent().unwrap()).unwrap();
        std::fs::write(&backup_schema, serialize_binary(&build_schema_doc(2124490))).unwrap();

        let parsed = load_schema_for_app(&steam, &data, 2124490).expect("backup schema must load");
        assert_eq!(parsed.source, SchemaSource::AetherBackup);
        assert_eq!(parsed.achievements.len(), 3);
    }

    #[test]
    fn icon_urls_normalize_filenames_and_full_urls() {
        assert_eq!(
            resolve_icon_url(2124490, "abc.jpg").as_deref(),
            Some("https://cdn.fastly.steamstatic.com/steamcommunity/public/images/apps/2124490/abc.jpg")
        );
        assert_eq!(
            resolve_icon_url(2124490, "https://example.com/x.jpg").as_deref(),
            Some("https://example.com/x.jpg")
        );
        assert_eq!(resolve_icon_url(2124490, "  "), None);
    }

    #[test]
    fn missing_schema_message_mentions_if_game_has_achievements() {
        let tmp = TempDir::new().unwrap();
        let err = load_schema_for_app(tmp.path(), &tmp.path().join("data"), 294100)
            .expect_err("missing schema must error");
        assert!(err.ends_with("if the game has achievements."));
    }
}
