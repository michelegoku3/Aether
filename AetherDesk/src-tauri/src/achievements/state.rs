//! User achievement & stat cache reader and reconciliation across:
//!   1. `<Steam>/appcache/stats/UserGameStats_<account>_<appid>.bin`
//!   2. `<AetherData>/backup/<appid>/achievements/UserGameStats_<account>_<appid>.bin`
//!   3. `<AetherData>/backup/<appid>/achievements/UserGameStats_<account>_<appid>.json`

use crate::achievements::kv1::{parse_binary, KvDict, KvValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StateSources {
    pub steam_bin_present: bool,
    pub backup_bin_present: bool,
    pub backup_json_present: bool,
    pub steam_unlock_count: usize,
    pub backup_unlock_count: usize,
    pub merged_from_backup: bool,
    pub pending_changes: i32,
    pub crc: i32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedStatsBin {
    pub crc: i32,
    pub pending_changes: i32,
    pub unlocked: BTreeMap<u32, u32>,
    pub stat_values: BTreeMap<u32, f32>,
    pub raw_stats: BTreeMap<u32, u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReconciledUserState {
    pub unlocked: BTreeMap<u32, u32>,
    pub stat_values: BTreeMap<u32, f32>,
    pub sources: StateSources,
}

pub fn steam_user_bin_path(steam_root: &Path, account_id: u32, app_id: u32) -> PathBuf {
    steam_root
        .join("appcache")
        .join("stats")
        .join(format!("UserGameStats_{account_id}_{app_id}.bin"))
}

pub fn backup_dir_for_app(data_root: &Path, app_id: u32) -> PathBuf {
    data_root
        .join("backup")
        .join(app_id.to_string())
        .join("achievements")
}

pub fn backup_user_bin_path(data_root: &Path, account_id: u32, app_id: u32) -> PathBuf {
    backup_dir_for_app(data_root, app_id).join(format!("UserGameStats_{account_id}_{app_id}.bin"))
}

pub fn backup_user_json_path(data_root: &Path, account_id: u32, app_id: u32) -> PathBuf {
    backup_dir_for_app(data_root, app_id).join(format!("UserGameStats_{account_id}_{app_id}.json"))
}

fn int32_field(dict: &KvDict, key: &str) -> Option<i32> {
    match dict.get_ci(key) {
        Some(KvValue::Int32(n)) => Some(*n),
        _ => None,
    }
}

fn merge_unlock_entry(target: &mut BTreeMap<u32, u32>, id: u32, unlock_time: u32) {
    match target.get_mut(&id) {
        None => {
            target.insert(id, unlock_time);
        }
        Some(existing) => {
            // Earliest non-zero observation wins, matching AchievementUserState::MergeUserState.
            if unlock_time != 0 && (*existing == 0 || unlock_time < *existing) {
                *existing = unlock_time;
            }
        }
    }
}

pub fn parse_user_stats_bin(
    bytes: &[u8],
    achievement_buckets: &BTreeSet<u32>,
) -> Result<ParsedStatsBin, String> {
    let doc = parse_binary(bytes)?;
    let Some(cache) = doc.root.get_ci("cache").and_then(|v| match v {
        KvValue::Dict(d) => Some(d),
        _ => None,
    }) else {
        return Err("UserGameStats binary stream missing `cache` root dictionary".to_string());
    };

    let mut out = ParsedStatsBin {
        crc: int32_field(cache, "crc").unwrap_or(0),
        pending_changes: int32_field(cache, "PendingChanges").unwrap_or(0),
        ..Default::default()
    };

    for (key, value) in &cache.entries {
        let Ok(bucket_id) = key.parse::<u32>() else {
            continue;
        };
        let KvValue::Dict(bucket_dict) = value else {
            continue;
        };

        let times_dict = bucket_dict.get_dict("AchievementTimes");
        let is_achievement_bucket =
            achievement_buckets.contains(&bucket_id) || times_dict.is_some();

        if is_achievement_bucket {
            let mut times = BTreeMap::new();
            if let Some(td) = times_dict {
                for (bit_key, t_val) in &td.entries {
                    let Ok(bit) = bit_key.parse::<u32>() else {
                        continue;
                    };
                    if bit >= 32 {
                        continue;
                    }
                    if let KvValue::Int32(ts) = t_val {
                        if *ts > 0 {
                            times.insert(bit, *ts as u32);
                        }
                    }
                }
            }

            let data_mask = int32_field(bucket_dict, "data").unwrap_or(0) as u32;
            let pending_mask = int32_field(bucket_dict, "pendingbits").unwrap_or(0) as u32;
            let union_mask = data_mask | pending_mask;
            for bit in 0..32u32 {
                if (union_mask & (1u32 << bit)) != 0 {
                    let ts = times.get(&bit).copied().unwrap_or(0);
                    merge_unlock_entry(&mut out.unlocked, bucket_id * 32 + bit, ts);
                }
            }
            for (bit, ts) in times {
                merge_unlock_entry(&mut out.unlocked, bucket_id * 32 + bit, ts);
            }
        } else if let Some(data_val) = bucket_dict.get("data") {
            match data_val {
                KvValue::Int32(n) => {
                    out.stat_values.insert(bucket_id, *n as f32);
                    out.raw_stats.insert(bucket_id, *n as u32);
                }
                KvValue::Float32(f) => {
                    out.stat_values.insert(bucket_id, *f);
                    out.raw_stats.insert(bucket_id, f.to_bits());
                }
                _ => {}
            }
        }
    }

    Ok(out)
}

#[derive(Deserialize)]
struct SnapshotUnlockJson {
    id: u32,
    #[serde(default)]
    unlock_time: u32,
}

#[derive(Deserialize)]
struct SnapshotStatJson {
    sid: u32,
    #[serde(default)]
    value: u32,
}

#[derive(Deserialize)]
struct SnapshotMirrorJson {
    #[serde(default)]
    achievements: Vec<SnapshotUnlockJson>,
    #[serde(default)]
    stats: Vec<SnapshotStatJson>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedBackupJson {
    pub unlocks: BTreeMap<u32, u32>,
    pub stats: BTreeMap<u32, u32>,
}

pub fn parse_backup_json(text: &str) -> Result<ParsedBackupJson, String> {
    let parsed: SnapshotMirrorJson = serde_json::from_str(text)
        .map_err(|e| format!("Failed to parse achievement backup JSON: {e}"))?;
    let mut unlocks = BTreeMap::new();
    for entry in parsed.achievements {
        merge_unlock_entry(&mut unlocks, entry.id, entry.unlock_time);
    }
    let mut stats = BTreeMap::new();
    for st in parsed.stats {
        stats.insert(st.sid, st.value);
    }
    Ok(ParsedBackupJson { unlocks, stats })
}

fn read_bin_if_present(
    path: &Path,
    label: &str,
    app_id: u32,
    account_id: u32,
    achievement_buckets: &BTreeSet<u32>,
) -> Option<ParsedStatsBin> {
    match fs::read(path) {
        Ok(bytes) if !bytes.is_empty() => match parse_user_stats_bin(&bytes, achievement_buckets) {
            Ok(parsed) => {
                crate::desk_log_debug!(
                    "achievements",
                    "Read {label} app_id={} account_id={} path={} bytes={} unlocks={} pending={}",
                    app_id,
                    account_id,
                    path.display(),
                    bytes.len(),
                    parsed.unlocked.len(),
                    parsed.pending_changes
                );
                Some(parsed)
            }
            Err(err) => {
                crate::desk_log_warn!(
                    "achievements",
                    "Corrupt {label} app_id={} account_id={} path={}: {}",
                    app_id,
                    account_id,
                    path.display(),
                    err
                );
                None
            }
        },
        Ok(_) => {
            crate::desk_log_debug!(
                "achievements",
                "Empty {label} app_id={} account_id={} path={}",
                app_id,
                account_id,
                path.display()
            );
            None
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            crate::desk_log_warn!(
                "achievements",
                "Cannot read {label} app_id={} account_id={} path={}: {}",
                app_id,
                account_id,
                path.display(),
                err
            );
            None
        }
    }
}

pub fn load_reconciled_user_state(
    steam_root: &Path,
    data_root: &Path,
    app_id: u32,
    account_id: u32,
    achievement_buckets: &BTreeSet<u32>,
) -> ReconciledUserState {
    let steam_path = steam_user_bin_path(steam_root, account_id, app_id);
    let backup_bin_path = backup_user_bin_path(data_root, account_id, app_id);
    let backup_json_path = backup_user_json_path(data_root, account_id, app_id);

    let steam_bin = read_bin_if_present(
        &steam_path,
        "steam_bin",
        app_id,
        account_id,
        achievement_buckets,
    );
    let backup_bin = read_bin_if_present(
        &backup_bin_path,
        "backup_bin",
        app_id,
        account_id,
        achievement_buckets,
    );
    let backup_json = match fs::read_to_string(&backup_json_path) {
        Ok(text) if !text.trim().is_empty() => match parse_backup_json(&text) {
            Ok(parsed) => {
                crate::desk_log_debug!(
                    "achievements",
                    "Read backup_json app_id={} account_id={} path={} unlocks={} stats={}",
                    app_id,
                    account_id,
                    backup_json_path.display(),
                    parsed.unlocks.len(),
                    parsed.stats.len()
                );
                Some(parsed)
            }
            Err(err) => {
                crate::desk_log_warn!(
                    "achievements",
                    "Corrupt backup_json app_id={} account_id={} path={}: {}",
                    app_id,
                    account_id,
                    backup_json_path.display(),
                    err
                );
                None
            }
        },
        _ => None,
    };

    let mut out = ReconciledUserState::default();
    out.sources.steam_bin_present = steam_bin.is_some();
    out.sources.backup_bin_present = backup_bin.is_some();
    out.sources.backup_json_present = backup_json.is_some();

    let mut steam_ids = BTreeSet::new();
    if let Some(sb) = &steam_bin {
        out.sources.steam_unlock_count = sb.unlocked.len();
        out.sources.pending_changes = sb.pending_changes;
        out.sources.crc = sb.crc;
        for (&id, &ts) in &sb.unlocked {
            steam_ids.insert(id);
            merge_unlock_entry(&mut out.unlocked, id, ts);
        }
        for (&sid, &val) in &sb.stat_values {
            out.stat_values.insert(sid, val);
        }
    }

    let mut backup_ids = BTreeSet::new();
    if let Some(bb) = &backup_bin {
        for (&id, &ts) in &bb.unlocked {
            backup_ids.insert(id);
            merge_unlock_entry(&mut out.unlocked, id, ts);
        }
        for (&sid, &val) in &bb.stat_values {
            out.stat_values.entry(sid).or_insert(val);
        }
    }
    if let Some(parsed_json) = &backup_json {
        for (&id, &ts) in &parsed_json.unlocks {
            backup_ids.insert(id);
            merge_unlock_entry(&mut out.unlocked, id, ts);
        }
        for (&sid, &val) in &parsed_json.stats {
            if !achievement_buckets.contains(&sid) {
                out.stat_values.entry(sid).or_insert(val as f32);
            }
        }
    }

    out.sources.backup_unlock_count = backup_ids.len();
    out.sources.merged_from_backup = out.unlocked.keys().any(|id| !steam_ids.contains(id));

    crate::desk_log_info!(
        "achievements",
        "State reconciled app_id={} account_id={} steam_unlocks={} backup_unlocks={} effective_unlocks={} merged_from_backup={} stats={}",
        app_id,
        account_id,
        out.sources.steam_unlock_count,
        out.sources.backup_unlock_count,
        out.unlocked.len(),
        out.sources.merged_from_backup,
        out.stat_values.len()
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::achievements::kv1::{serialize_binary, KvDict, KvDocument, KvValue};
    use tempfile::TempDir;

    fn make_user_bin(
        pending: i32,
        data_mask: i32,
        pending_mask: i32,
        times: Vec<(&str, i32)>,
        counter_value: f32,
    ) -> Vec<u8> {
        let mut bucket0_entries = vec![
            ("data".to_string(), KvValue::Int32(data_mask)),
            ("pendingbits".to_string(), KvValue::Int32(pending_mask)),
            ("state".to_string(), KvValue::Int32(2)),
        ];
        if !times.is_empty() {
            bucket0_entries.push((
                "AchievementTimes".to_string(),
                KvValue::Dict(KvDict {
                    entries: times
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), KvValue::Int32(v)))
                        .collect(),
                }),
            ));
        }

        let doc = KvDocument {
            root: KvDict {
                entries: vec![(
                    "cache".to_string(),
                    KvValue::Dict(KvDict {
                        entries: vec![
                            ("crc".to_string(), KvValue::Int32(0)),
                            ("PendingChanges".to_string(), KvValue::Int32(pending)),
                            (
                                "0".to_string(),
                                KvValue::Dict(KvDict {
                                    entries: bucket0_entries,
                                }),
                            ),
                            (
                                "9".to_string(),
                                KvValue::Dict(KvDict {
                                    entries: vec![(
                                        "data".to_string(),
                                        KvValue::Float32(counter_value),
                                    )],
                                }),
                            ),
                        ],
                    }),
                )],
            },
            has_outer_end: true,
        };
        serialize_binary(&doc)
    }

    #[test]
    fn bin_parser_unions_data_pendingbits_and_achievement_times() {
        let buckets = BTreeSet::from([0]);
        // bit 0 in data, bit 1 in pendingbits, bit 2 only in AchievementTimes!
        let bytes = make_user_bin(1, 0b001, 0b010, vec![("0", 1000), ("2", 2000)], 6.0);
        let parsed = parse_user_stats_bin(&bytes, &buckets).expect("user bin must parse");
        assert_eq!(parsed.pending_changes, 1);
        assert_eq!(parsed.unlocked.get(&0), Some(&1000));
        assert_eq!(parsed.unlocked.get(&1), Some(&0));
        assert_eq!(parsed.unlocked.get(&2), Some(&2000));
        assert_eq!(parsed.stat_values.get(&9), Some(&6.0));
    }

    #[test]
    fn reconcile_merges_backup_unlocks_and_dates_when_steam_cache_was_wiped() {
        let tmp = TempDir::new().unwrap();
        let steam = tmp.path().join("steam");
        let data = tmp.path().join("AetherData");
        let stats_dir = steam.join("appcache").join("stats");
        let backup_dir = data.join("backup").join("2124490").join("achievements");
        std::fs::create_dir_all(&stats_dir).unwrap();
        std::fs::create_dir_all(&backup_dir).unwrap();

        // Steam bin lost unlock bit 1 and lost date for bit 0
        std::fs::write(
            stats_dir.join("UserGameStats_99_2124490.bin"),
            make_user_bin(0, 0b01, 0, vec![], 4.0),
        )
        .unwrap();
        // Backup bin still holds bit 0 + bit 1
        std::fs::write(
            backup_dir.join("UserGameStats_99_2124490.bin"),
            make_user_bin(1, 0b11, 0, vec![("0", 1700), ("1", 1800)], 4.0),
        )
        .unwrap();
        // Backup JSON adds earlier date for bit 0 and unlock id 4
        std::fs::write(
            backup_dir.join("UserGameStats_99_2124490.json"),
            r#"{
              "_format": "aether-achievement-mirror-v1",
              "appid": 2124490,
              "account_id": 99,
              "achievements": [
                {"id": 0, "unlock_time": 1600, "bucket": 0, "bit": 0},
                {"id": 4, "unlock_time": 1900, "bucket": 0, "bit": 4}
              ],
              "stats": [{"sid": 9, "value": 4}]
            }"#,
        )
        .unwrap();

        let state = load_reconciled_user_state(&steam, &data, 2124490, 99, &BTreeSet::from([0]));
        assert!(state.sources.steam_bin_present);
        assert!(state.sources.backup_bin_present);
        assert!(state.sources.backup_json_present);
        assert!(state.sources.merged_from_backup);
        assert_eq!(state.sources.steam_unlock_count, 1);
        assert_eq!(state.unlocked.get(&0), Some(&1600)); // earliest non-zero time wins
        assert_eq!(state.unlocked.get(&1), Some(&1800));
        assert_eq!(state.unlocked.get(&4), Some(&1900));
        assert_eq!(state.stat_values.get(&9), Some(&4.0));
    }
}
