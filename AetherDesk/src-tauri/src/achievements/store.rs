//! Transactional achievement editor and account resolver.
//!
//! Commits changes across all three synchronized locations:
//!   1. `<AetherData>/backup/<appid>/achievements/UserGameStats_<account>_<appid>.json`
//!   2. `<AetherData>/backup/<appid>/achievements/UserGameStats_<account>_<appid>.bin`
//!   3. `<Steam>/appcache/stats/UserGameStats_<account>_<appid>.bin`
//! Updating both backup and Steam cache intentionally keeps counts in sync so
//! AetherDLL's monotone guard (`SteamStatsCache.cpp`) never reverts explicit relocks.

use crate::achievements::kv1::{parse_binary, serialize_binary, KvDict, KvDocument, KvValue};
use crate::achievements::schema::{load_schema_for_app, resolve_icon_url, SchemaSource};
use crate::achievements::state::{
    backup_dir_for_app, backup_user_bin_path, backup_user_json_path, load_reconciled_user_state,
    parse_backup_json, parse_user_stats_bin, steam_user_bin_path, StateSources,
};
use crate::external_tools::fs::write_atomic;
use crate::steam::loginusers::{
    detect_active_steam_id, read_accounts, select_active_steam_id_from_vdf,
    steam_id64_from_account_id,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const STEAM_ID64_INDIVIDUAL_BASE: u64 = 76_561_197_960_265_728;

fn account_id_from_steam_id64(steam64: u64) -> Option<u32> {
    let id = steam64.checked_sub(STEAM_ID64_INDIVIDUAL_BASE)?;
    u32::try_from(id).ok().filter(|&v| v != 0)
}

pub const STEAM_RUNNING_ACHIEVEMENTS_ERROR: &str =
    "STEAM_RUNNING: Steam is currently running. Close Steam completely before saving achievement changes.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AchievementChange {
    pub id: u32,
    pub unlocked: bool,
    pub unlock_time: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AchievementAccountOption {
    pub account_id: u32,
    pub steam_id64: String,
    pub persona_name: String,
    pub is_active: bool,
    pub has_stats_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AchievementProgressView {
    pub current: f32,
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AchievementItemView {
    pub id: u32,
    pub bucket: u32,
    pub bit: u32,
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub hidden: bool,
    pub unlocked: bool,
    pub unlock_time: u32,
    pub icon_url: Option<String>,
    pub icon_gray_url: Option<String>,
    pub progress: Option<AchievementProgressView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAchievementsSnapshot {
    pub app_id: u32,
    pub game_name: String,
    pub schema_version: i32,
    pub schema_source: SchemaSource,
    pub lua_managed: bool,
    pub account_id: u32,
    pub accounts: Vec<AchievementAccountOption>,
    pub unlocked_count: usize,
    pub total_count: usize,
    pub sources: StateSources,
    pub achievements: Vec<AchievementItemView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAchievementsReport {
    pub app_id: u32,
    pub account_id: u32,
    pub unlocked_after: usize,
    pub total_count: usize,
    pub applied_unlocks: usize,
    pub applied_relocks: usize,
    pub lua_managed: bool,
}

pub fn is_app_lua_managed(steam_root: &Path, app_id: u32) -> bool {
    steam_root
        .join("config")
        .join("stplug-in")
        .join(format!("{app_id}.lua"))
        .is_file()
}

pub fn now_unix_seconds() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().min(u64::from(u32::MAX)) as u32)
        .unwrap_or(0)
}

fn scan_cached_accounts_in_dir(dir: &Path, app_id: u32, into: &mut BTreeSet<u32>) {
    let suffix_bin = format!("_{app_id}.bin");
    let suffix_json = format!("_{app_id}.json");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let text = name.to_string_lossy();
        let Some(rest) = text.strip_prefix("UserGameStats_") else {
            continue;
        };
        let account_str = rest
            .strip_suffix(&suffix_bin)
            .or_else(|| rest.strip_suffix(&suffix_json));
        if let Some(id_text) = account_str {
            if let Ok(account_id) = id_text.parse::<u32>() {
                if account_id != 0 {
                    into.insert(account_id);
                }
            }
        }
    }
}

fn resolve_accounts(
    steam_root: &Path,
    data_root: &Path,
    app_id: u32,
    requested_account_id: Option<u32>,
    registry_active_account_id: Option<u32>,
) -> Result<(u32, Vec<AchievementAccountOption>), String> {
    let login_accounts = read_accounts(steam_root);
    let mut cached_ids = BTreeSet::new();
    scan_cached_accounts_in_dir(
        &steam_root.join("appcache").join("stats"),
        app_id,
        &mut cached_ids,
    );
    scan_cached_accounts_in_dir(
        &backup_dir_for_app(data_root, app_id),
        app_id,
        &mut cached_ids,
    );

    // Build ordered map of known accounts from loginusers.vdf first, then any cached-only ids.
    let mut options: Vec<AchievementAccountOption> = Vec::new();
    let mut seen = BTreeSet::new();

    for acc in &login_accounts {
        let Ok(steam64) = acc.steam_id.parse::<u64>() else {
            continue;
        };
        let Some(acct_id) = account_id_from_steam_id64(steam64) else {
            continue;
        };
        if acct_id == 0 || !seen.insert(acct_id) {
            continue;
        }
        let label = if !acc.persona_name.trim().is_empty() {
            acc.persona_name.clone()
        } else if !acc.account_name.trim().is_empty() {
            acc.account_name.clone()
        } else {
            acct_id.to_string()
        };
        options.push(AchievementAccountOption {
            account_id: acct_id,
            steam_id64: acc.steam_id.clone(),
            persona_name: label,
            is_active: false,
            has_stats_cache: cached_ids.contains(&acct_id),
        });
    }

    for acct_id in cached_ids {
        if seen.insert(acct_id) {
            options.push(AchievementAccountOption {
                account_id: acct_id,
                steam_id64: steam_id64_from_account_id(acct_id),
                persona_name: format!("Account {acct_id}"),
                is_active: false,
                has_stats_cache: true,
            });
        }
    }

    // Active preference: if the VDF defines accounts, pick MostRecent/active within the Steam tree first;
    // only use registry active user when it belongs to this Steam tree or loginusers.vdf is absent.
    let vdf_active = crate::steam::loginusers::read_accounts(steam_root);
    let vdf_most_recent = fs::read_to_string(steam_root.join("config").join("loginusers.vdf"))
        .ok()
        .and_then(|txt| select_active_steam_id_from_vdf(&txt))
        .and_then(|s| s.parse::<u64>().ok())
        .and_then(account_id_from_steam_id64);

    let active_id = if let Some(reg_id) = registry_active_account_id {
        if vdf_active.is_empty() || seen.contains(&reg_id) {
            Some(reg_id)
        } else {
            vdf_most_recent.or_else(|| options.first().map(|o| o.account_id))
        }
    } else {
        vdf_most_recent.or_else(|| options.first().map(|o| o.account_id))
    };

    if let Some(aid) = active_id {
        if !seen.contains(&aid) {
            options.insert(
                0,
                AchievementAccountOption {
                    account_id: aid,
                    steam_id64: steam_id64_from_account_id(aid),
                    persona_name: format!("Account {aid}"),
                    is_active: true,
                    has_stats_cache: false,
                },
            );
        }
        for opt in &mut options {
            opt.is_active = opt.account_id == aid;
        }
    }

    let selected = match requested_account_id {
        Some(id) if id != 0 => {
            if !options.iter().any(|o| o.account_id == id) {
                options.push(AchievementAccountOption {
                    account_id: id,
                    steam_id64: steam_id64_from_account_id(id),
                    persona_name: format!("Account {id}"),
                    is_active: false,
                    has_stats_cache: false,
                });
            }
            id
        }
        _ => active_id
            .or_else(|| {
                options
                    .iter()
                    .find(|o| o.has_stats_cache)
                    .map(|o| o.account_id)
            })
            .or_else(|| options.first().map(|o| o.account_id))
            .ok_or_else(|| {
                format!(
                    "NO_ACCOUNT: Could not determine any local Steam account for AppID {app_id}."
                )
            })?,
    };

    Ok((selected, options))
}

fn registry_active_account(steam_root: &Path) -> Option<u32> {
    detect_active_steam_id(steam_root).and_then(|(steam64, source)| {
        if source == "registry" {
            steam64
                .parse::<u64>()
                .ok()
                .and_then(account_id_from_steam_id64)
        } else {
            None
        }
    })
}

pub fn inspect_game_achievements(
    steam_root: &Path,
    data_root: &Path,
    app_id: u32,
    requested_account_id: Option<u32>,
) -> Result<GameAchievementsSnapshot, String> {
    if app_id == 0 {
        return Err("A valid AppID is required to inspect achievements.".to_string());
    }
    let reg_active = registry_active_account(steam_root);
    let schema = load_schema_for_app(steam_root, data_root, app_id)?;
    let (account_id, accounts) = resolve_accounts(
        steam_root,
        data_root,
        app_id,
        requested_account_id,
        reg_active,
    )?;
    let state = load_reconciled_user_state(
        steam_root,
        data_root,
        app_id,
        account_id,
        &schema.achievement_buckets,
    );
    let lua_managed = is_app_lua_managed(steam_root, app_id);

    let mut unlocked_count = 0usize;
    let mut achievements = Vec::with_capacity(schema.achievements.len());
    for item in &schema.achievements {
        let unlock_time = state.unlocked.get(&item.id).copied();
        let unlocked = unlock_time.is_some();
        if unlocked {
            unlocked_count += 1;
        }
        let progress = item
            .progress_stat
            .as_ref()
            .map(|ps| AchievementProgressView {
                current: state.stat_values.get(&ps.stat_id).copied().unwrap_or(0.0),
                min: ps.min_val,
                max: ps.max_val,
            });
        achievements.push(AchievementItemView {
            id: item.id,
            bucket: item.bucket,
            bit: item.bit,
            api_name: item.api_name.clone(),
            name: item.name.clone(),
            description: item.description.clone(),
            hidden: item.hidden,
            unlocked,
            unlock_time: unlock_time.unwrap_or(0),
            icon_url: resolve_icon_url(app_id, &item.icon),
            icon_gray_url: resolve_icon_url(app_id, &item.icon_gray),
            progress,
        });
    }

    let game_name = schema.game_name.unwrap_or_else(|| format!("App {app_id}"));

    crate::desk_log_info!(
        "achievements",
        "Inspect complete app_id={} account_id={} lua_managed={} unlocked={}/{} source={:?}",
        app_id,
        account_id,
        lua_managed,
        unlocked_count,
        achievements.len(),
        schema.source
    );

    Ok(GameAchievementsSnapshot {
        app_id,
        game_name,
        schema_version: schema.version,
        schema_source: schema.source,
        lua_managed,
        account_id,
        accounts,
        unlocked_count,
        total_count: achievements.len(),
        sources: state.sources,
        achievements,
    })
}

pub fn patch_user_stats_kv1(
    existing_bytes: Option<&[u8]>,
    achievement_buckets: &BTreeSet<u32>,
    desired_unlocks: &BTreeMap<u32, u32>,
    changed_buckets: &BTreeSet<u32>,
) -> Result<Vec<u8>, String> {
    let mut doc = match existing_bytes {
        Some(bytes) if !bytes.is_empty() => parse_binary(bytes)?,
        _ => KvDocument {
            root: KvDict {
                entries: vec![(
                    "cache".to_string(),
                    KvValue::Dict(KvDict {
                        entries: vec![
                            ("crc".to_string(), KvValue::Int32(0)),
                            ("PendingChanges".to_string(), KvValue::Int32(1)),
                        ],
                    }),
                )],
            },
            has_outer_end: true,
        },
    };

    // Determine all achievement buckets that must be synchronized:
    // anything edited plus any existing achievement bucket in cache.
    let mut target_buckets = changed_buckets.clone();
    if let Some(cache) = doc.root.get_dict("cache") {
        for (k, _) in &cache.entries {
            if let Ok(bucket_id) = k.parse::<u32>() {
                if achievement_buckets.contains(&bucket_id) {
                    target_buckets.insert(bucket_id);
                }
            }
        }
    }

    let cache = doc.root.get_or_insert_dict_mut("cache");
    cache.set("crc", KvValue::Int32(0));
    cache.set("PendingChanges", KvValue::Int32(1));

    for bucket_id in target_buckets {
        let mut mask: u32 = 0;
        let mut bucket_times: BTreeMap<u32, u32> = BTreeMap::new();
        for bit in 0..32u32 {
            let aid = bucket_id * 32 + bit;
            if let Some(&ts) = desired_unlocks.get(&aid) {
                mask |= 1u32 << bit;
                if ts > 0 {
                    bucket_times.insert(bit, ts);
                }
            }
        }

        let bucket_key = bucket_id.to_string();
        let bucket_dict = cache.get_or_insert_dict_mut(&bucket_key);
        let prev_data = match bucket_dict.get("data") {
            Some(KvValue::Int32(n)) => *n as u32,
            _ => 0,
        };
        let had_pending = bucket_dict.get("pendingbits").is_some();
        let prev_pending = match bucket_dict.get("pendingbits") {
            Some(KvValue::Int32(n)) => *n as u32,
            _ => 0,
        };

        // Keep previously pending bits that remain unlocked, add newly unlocked bits,
        // and strictly clear any relocked bits from both `data` and `pendingbits`.
        let newly_added = mask & !prev_data;
        let next_pending = (prev_pending & mask) | newly_added;

        bucket_dict.set("data", KvValue::Int32(mask as i32));
        if had_pending || next_pending != 0 {
            bucket_dict.set("pendingbits", KvValue::Int32(next_pending as i32));
        }
        if bucket_dict.get("state").is_none() {
            bucket_dict.set("state", KvValue::Int32(2));
        }

        let times_dict = bucket_dict.get_or_insert_dict_mut("AchievementTimes");
        // Drop any timestamps for bits that are no longer unlocked, then write/keep unlocked timestamps.
        times_dict.entries.retain(|(bit_key, _)| {
            bit_key
                .parse::<u32>()
                .ok()
                .map(|b| b < 32 && (mask & (1u32 << b)) != 0)
                .unwrap_or(true)
        });
        for (bit, ts) in bucket_times {
            let bit_key = bit.to_string();
            let clamped = ts.min(i32::MAX as u32) as i32;
            times_dict.set(bit_key, KvValue::Int32(clamped));
        }
    }

    Ok(serialize_binary(&doc))
}

fn build_mirror_json(
    app_id: u32,
    account_id: u32,
    unlocked: &BTreeMap<u32, u32>,
    preserved_stats: &BTreeMap<u32, u32>,
    now_unix: u32,
) -> String {
    let steam64 = steam_id64_from_account_id(account_id);
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"_format\": \"aether-achievement-mirror-v1\",\n");
    out.push_str("  \"_note\": \"snapshot synced by AetherDesk; rebuild the .bin with Tools/achievement_decoder.py rebuild\",\n");
    out.push_str(&format!("  \"appid\": {app_id},\n"));
    out.push_str(&format!("  \"account_id\": {account_id},\n"));
    out.push_str(&format!("  \"steamid64\": {steam64},\n"));
    out.push_str(&format!("  \"updated\": \"unix:{now_unix}\",\n"));
    out.push_str(&format!("  \"updated_unix\": {now_unix},\n"));
    out.push_str("  \"achievements\": [\n");
    let total = unlocked.len();
    for (idx, (&id, &unlock_time)) in unlocked.iter().enumerate() {
        let bucket = id / 32;
        let bit = id % 32;
        let comma = if idx + 1 < total { "," } else { "" };
        out.push_str(&format!(
            "    {{\"id\": {id}, \"unlock_time\": {unlock_time}, \"unlocked_at\": \"{}\", \"bucket\": {bucket}, \"bit\": {bit}}}{comma}\n",
            if unlock_time != 0 {
                format!("unix:{unlock_time}")
            } else {
                "unknown".to_string()
            }
        ));
    }
    out.push_str("  ],\n");
    out.push_str("  \"stats\": [\n");
    let st_total = preserved_stats.len();
    for (idx, (&sid, &val)) in preserved_stats.iter().enumerate() {
        let comma = if idx + 1 < st_total { "," } else { "" };
        out.push_str(&format!(
            "    {{\"sid\": {sid}, \"value\": {val}}}{comma}\n"
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

struct FileRollback {
    path: PathBuf,
    previous: Option<Vec<u8>>,
}

impl FileRollback {
    fn capture(path: PathBuf) -> Result<Self, String> {
        let previous = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("Cannot snapshot {}: {e}", path.display())),
        };
        Ok(Self { path, previous })
    }

    fn restore(&self) {
        match &self.previous {
            Some(bytes) => {
                if let Err(err) = write_atomic(&self.path, bytes) {
                    crate::desk_log_warn!(
                        "achievements",
                        "Rollback failed for {}: {}",
                        self.path.display(),
                        err
                    );
                }
            }
            None => {
                if self.path.exists() {
                    if let Err(err) = fs::remove_file(&self.path) {
                        crate::desk_log_warn!(
                            "achievements",
                            "Rollback remove failed for {}: {}",
                            self.path.display(),
                            err
                        );
                    }
                }
            }
        }
    }
}

pub fn commit_achievement_changes_blocking(
    steam_root: &Path,
    data_root: &Path,
    app_id: u32,
    account_id: u32,
    changes: &[AchievementChange],
    allow_steam_kill: bool,
    now_unix: u32,
    steam_gate_fn: fn(&Path, bool) -> Result<(), String>,
) -> Result<SaveAchievementsReport, String> {
    commit_achievement_changes_with_writer(
        steam_root,
        data_root,
        app_id,
        account_id,
        changes,
        allow_steam_kill,
        now_unix,
        steam_gate_fn,
        |path, bytes| write_atomic(path, bytes),
    )
}

#[allow(clippy::too_many_arguments)]
fn commit_achievement_changes_with_writer<F>(
    steam_root: &Path,
    data_root: &Path,
    app_id: u32,
    account_id: u32,
    changes: &[AchievementChange],
    allow_steam_kill: bool,
    now_unix: u32,
    steam_gate_fn: fn(&Path, bool) -> Result<(), String>,
    mut write_fn: F,
) -> Result<SaveAchievementsReport, String>
where
    F: FnMut(&Path, &[u8]) -> Result<(), String>,
{
    if app_id == 0 || account_id == 0 {
        return Err("Valid AppID and Steam account ID are required.".to_string());
    }
    if changes.is_empty() {
        return Err("No achievement changes were requested.".to_string());
    }

    let schema = load_schema_for_app(steam_root, data_root, app_id)?;
    let valid_ids: BTreeSet<u32> = schema.achievements.iter().map(|a| a.id).collect();
    for ch in changes {
        if !valid_ids.contains(&ch.id) {
            return Err(format!(
                "Achievement id {} is not defined in schema for AppID {app_id}.",
                ch.id
            ));
        }
    }

    // 1. Refuse or close Steam BEFORE reading final live disk state / patching.
    steam_gate_fn(steam_root, allow_steam_kill)?;

    // 2. Post-close fresh read: catches any flush Steam wrote just before exiting.
    let steam_bin_path = steam_user_bin_path(steam_root, account_id, app_id);
    let backup_bin_path = backup_user_bin_path(data_root, account_id, app_id);
    let backup_json_path = backup_user_json_path(data_root, account_id, app_id);

    let steam_bytes = fs::read(&steam_bin_path).ok().filter(|b| !b.is_empty());
    let backup_bytes = fs::read(&backup_bin_path).ok().filter(|b| !b.is_empty());
    let reconciled = load_reconciled_user_state(
        steam_root,
        data_root,
        app_id,
        account_id,
        &schema.achievement_buckets,
    );

    let mut desired_unlocks = reconciled.unlocked.clone();
    let mut changed_buckets = BTreeSet::new();
    let mut applied_unlocks = 0usize;
    let mut applied_relocks = 0usize;

    for ch in changes {
        changed_buckets.insert(ch.id / 32);
        let was_unlocked = desired_unlocks.contains_key(&ch.id);
        if ch.unlocked {
            let ts = ch
                .unlock_time
                .filter(|&t| t > 0)
                .or_else(|| desired_unlocks.get(&ch.id).copied().filter(|&t| t > 0))
                .unwrap_or(now_unix);
            desired_unlocks.insert(ch.id, ts);
            if !was_unlocked {
                applied_unlocks += 1;
            }
        } else if desired_unlocks.remove(&ch.id).is_some() {
            applied_relocks += 1;
        }
    }

    // Preserve raw int/float stat buckets from the freshest binary cache or JSON.
    let mut preserved_stats = BTreeMap::new();
    if let Some(bytes) = steam_bytes.as_deref().or(backup_bytes.as_deref()) {
        if let Ok(parsed) = parse_user_stats_bin(bytes, &schema.achievement_buckets) {
            preserved_stats = parsed.raw_stats;
        }
    }
    if let Ok(text) = fs::read_to_string(&backup_json_path) {
        if let Ok((_, json_stats)) = parse_backup_json(&text) {
            for (sid, val) in json_stats {
                if !schema.achievement_buckets.contains(&sid) {
                    preserved_stats.entry(sid).or_insert(val);
                }
            }
        }
    }

    // Patch from steam_bytes if present & valid, else fallback to backup_bytes, else fresh KV1 cache.
    let base_bytes = match steam_bytes.as_deref() {
        Some(b) if parse_binary(b).is_ok() => Some(b),
        _ => match backup_bytes.as_deref() {
            Some(b) if parse_binary(b).is_ok() => Some(b),
            _ => None,
        },
    };

    let patched_bin = patch_user_stats_kv1(
        base_bytes,
        &schema.achievement_buckets,
        &desired_unlocks,
        &changed_buckets,
    )?;
    let mirror_json = build_mirror_json(
        app_id,
        account_id,
        &desired_unlocks,
        &preserved_stats,
        now_unix,
    );

    if let Some(parent) = backup_json_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create achievement backup folder {}: {e}",
                parent.display()
            )
        })?;
    }
    if let Some(parent) = steam_bin_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create Steam stats cache folder {}: {e}",
                parent.display()
            )
        })?;
    }

    // 3. Capture rollback state and commit atomically across all 3 targets.
    let rb_json = FileRollback::capture(backup_json_path.clone())?;
    let rb_bbin = FileRollback::capture(backup_bin_path.clone())?;
    let rb_sbin = FileRollback::capture(steam_bin_path.clone())?;

    if let Err(err) = write_fn(&backup_json_path, mirror_json.as_bytes()) {
        crate::desk_log_warn!("achievements", "Commit aborted on backup JSON write: {err}");
        return Err(err);
    }
    if let Err(err) = write_fn(&backup_bin_path, &patched_bin) {
        crate::desk_log_warn!("achievements", "Commit aborted on backup BIN write: {err}");
        rb_json.restore();
        return Err(err);
    }
    if let Err(err) = write_fn(&steam_bin_path, &patched_bin) {
        crate::desk_log_warn!(
            "achievements",
            "Commit aborted on Steam BIN write (rolling back backups): {err}"
        );
        rb_bbin.restore();
        rb_json.restore();
        rb_sbin.restore();
        return Err(err);
    }

    let lua_managed = is_app_lua_managed(steam_root, app_id);
    crate::desk_log_info!(
        "achievements",
        "Committed achievement changes app_id={} account_id={} unlocked={}/{} +unlocks={} -relocks={} lua_managed={} bin_bytes={}",
        app_id,
        account_id,
        desired_unlocks.len(),
        schema.achievements.len(),
        applied_unlocks,
        applied_relocks,
        lua_managed,
        patched_bin.len()
    );

    Ok(SaveAchievementsReport {
        app_id,
        account_id,
        unlocked_after: desired_unlocks.len(),
        total_count: schema.achievements.len(),
        applied_unlocks,
        applied_relocks,
        lua_managed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::achievements::kv1::{parse_binary, serialize_binary, KvDict, KvDocument, KvValue};
    use crate::achievements::state::{
        backup_user_bin_path, backup_user_json_path, parse_backup_json, parse_user_stats_bin,
        steam_user_bin_path,
    };
    use tempfile::TempDir;

    fn kv_dict(entries: Vec<(&str, KvValue)>) -> KvValue {
        KvValue::Dict(KvDict {
            entries: entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        })
    }

    fn write_test_schema(steam_root: &Path, app_id: u32) {
        let doc = KvDocument {
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
                                                            KvValue::Str("ACH_ONE".to_string()),
                                                        ),
                                                        (
                                                            "display",
                                                            kv_dict(vec![
                                                                (
                                                                    "name",
                                                                    kv_dict(vec![(
                                                                        "english",
                                                                        KvValue::Str(
                                                                            "One".to_string(),
                                                                        ),
                                                                    )]),
                                                                ),
                                                                (
                                                                    "desc",
                                                                    kv_dict(vec![(
                                                                        "english",
                                                                        KvValue::Str(
                                                                            "First".to_string(),
                                                                        ),
                                                                    )]),
                                                                ),
                                                                ("hidden", KvValue::Int32(0)),
                                                                (
                                                                    "icon",
                                                                    KvValue::Str(
                                                                        "one.jpg".to_string(),
                                                                    ),
                                                                ),
                                                                (
                                                                    "icon_gray",
                                                                    KvValue::Str(
                                                                        "one_g.jpg".to_string(),
                                                                    ),
                                                                ),
                                                            ]),
                                                        ),
                                                    ]),
                                                ),
                                                (
                                                    "1",
                                                    kv_dict(vec![
                                                        (
                                                            "name",
                                                            KvValue::Str("ACH_TWO".to_string()),
                                                        ),
                                                        (
                                                            "display",
                                                            kv_dict(vec![
                                                                (
                                                                    "name",
                                                                    kv_dict(vec![(
                                                                        "english",
                                                                        KvValue::Str(
                                                                            "Two".to_string(),
                                                                        ),
                                                                    )]),
                                                                ),
                                                                ("hidden", KvValue::Int32(1)),
                                                            ]),
                                                        ),
                                                    ]),
                                                ),
                                            ]),
                                        ),
                                    ]),
                                ),
                                (
                                    "5",
                                    kv_dict(vec![
                                        ("name", KvValue::Str("KILLS".to_string())),
                                        ("type", KvValue::Str("FLOAT".to_string())),
                                    ]),
                                ),
                            ]),
                        ),
                        ("version", KvValue::Int32(3)),
                        ("gamename", KvValue::Str("Demo Game".to_string())),
                    ]),
                )],
            },
            has_outer_end: true,
        };
        let path = crate::achievements::schema::steam_schema_path(steam_root, app_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serialize_binary(&doc)).unwrap();
    }

    fn write_loginusers(steam_root: &Path, account_id: u32) {
        let steam64 = 76561197960265728u64 + u64::from(account_id);
        let cfg = steam_root.join("config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(
            cfg.join("loginusers.vdf"),
            format!(
                "\"users\"\n{{\n\t\"{steam64}\"\n\t{{\n\t\t\"AccountName\"\t\t\"player1\"\n\t\t\"PersonaName\"\t\t\"Player One\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}}\n}}\n"
            ),
        )
        .unwrap();
    }

    fn make_existing_bin(data_mask: i32, pending_mask: i32, times: Vec<(&str, i32)>) -> Vec<u8> {
        let doc = KvDocument {
            root: KvDict {
                entries: vec![(
                    "cache".to_string(),
                    KvValue::Dict(KvDict {
                        entries: vec![
                            ("crc".to_string(), KvValue::Int32(42)),
                            ("PendingChanges".to_string(), KvValue::Int32(0)),
                            (
                                "0".to_string(),
                                kv_dict(vec![
                                    ("data", KvValue::Int32(data_mask)),
                                    ("pendingbits", KvValue::Int32(pending_mask)),
                                    ("state", KvValue::Int32(2)),
                                    (
                                        "AchievementTimes",
                                        kv_dict(
                                            times
                                                .into_iter()
                                                .map(|(k, v)| (k, KvValue::Int32(v)))
                                                .collect(),
                                        ),
                                    ),
                                ]),
                            ),
                            (
                                "5".to_string(),
                                kv_dict(vec![("data", KvValue::Float32(42.5))]),
                            ),
                        ],
                    }),
                )],
            },
            has_outer_end: true,
        };
        serialize_binary(&doc)
    }

    fn noop_gate(_steam: &Path, _allow: bool) -> Result<(), String> {
        Ok(())
    }

    fn refuse_gate(_steam: &Path, allow: bool) -> Result<(), String> {
        if allow {
            Ok(())
        } else {
            Err(STEAM_RUNNING_ACHIEVEMENTS_ERROR.to_string())
        }
    }

    #[test]
    fn unlock_preserves_unrelated_stat_buckets_and_writes_all_three_targets() {
        let tmp = TempDir::new().unwrap();
        let steam = tmp.path().join("steam");
        let data = tmp.path().join("AetherData");
        let acct = 920691037u32;
        let app = 2124490u32;

        write_test_schema(&steam, app);
        write_loginusers(&steam, acct);
        std::fs::create_dir_all(steam.join("config").join("stplug-in")).unwrap();
        std::fs::write(
            steam
                .join("config")
                .join("stplug-in")
                .join(format!("{app}.lua")),
            "addappid(2124490)\n",
        )
        .unwrap();

        let steam_bin = steam_user_bin_path(&steam, acct, app);
        std::fs::create_dir_all(steam_bin.parent().unwrap()).unwrap();
        std::fs::write(&steam_bin, make_existing_bin(0, 0, vec![])).unwrap();

        let report = commit_achievement_changes_blocking(
            &steam,
            &data,
            app,
            acct,
            &[AchievementChange {
                id: 1,
                unlocked: true,
                unlock_time: Some(1_760_000_111),
            }],
            false,
            1_760_000_999,
            noop_gate,
        )
        .expect("commit should succeed");

        assert_eq!(report.unlocked_after, 1);
        assert_eq!(report.applied_unlocks, 1);
        assert!(report.lua_managed);

        let steam_parsed =
            parse_user_stats_bin(&std::fs::read(&steam_bin).unwrap(), &BTreeSet::from([0]))
                .unwrap();
        let backup_parsed = parse_user_stats_bin(
            &std::fs::read(backup_user_bin_path(&data, acct, app)).unwrap(),
            &BTreeSet::from([0]),
        )
        .unwrap();
        assert_eq!(steam_parsed.unlocked.get(&1), Some(&1_760_000_111));
        assert_eq!(backup_parsed.unlocked, steam_parsed.unlocked);
        // Float stat bucket 5 must be untouched!
        assert_eq!(steam_parsed.stat_values.get(&5), Some(&42.5));

        let (json_unlocks, _) = parse_backup_json(
            &std::fs::read_to_string(backup_user_json_path(&data, acct, app)).unwrap(),
        )
        .unwrap();
        assert_eq!(json_unlocks.get(&1), Some(&1_760_000_111));
    }

    #[test]
    fn relock_clears_data_pendingbits_times_and_both_backup_files() {
        let tmp = TempDir::new().unwrap();
        let steam = tmp.path().join("steam");
        let data = tmp.path().join("AetherData");
        let acct = 99u32;
        let app = 2124490u32;

        write_test_schema(&steam, app);
        write_loginusers(&steam, acct);

        let initial_bin = make_existing_bin(0b11, 0b10, vec![("0", 100), ("1", 200)]);
        let sb = steam_user_bin_path(&steam, acct, app);
        let bb = backup_user_bin_path(&data, acct, app);
        let bj = backup_user_json_path(&data, acct, app);
        std::fs::create_dir_all(sb.parent().unwrap()).unwrap();
        std::fs::create_dir_all(bb.parent().unwrap()).unwrap();
        std::fs::write(&sb, &initial_bin).unwrap();
        std::fs::write(&bb, &initial_bin).unwrap();
        std::fs::write(
            &bj,
            r#"{"_format":"aether-achievement-mirror-v1","appid":2124490,"account_id":99,"achievements":[{"id":0,"unlock_time":100},{"id":1,"unlock_time":200}],"stats":[]}"#,
        )
        .unwrap();

        let report = commit_achievement_changes_blocking(
            &steam,
            &data,
            app,
            acct,
            &[AchievementChange {
                id: 1,
                unlocked: false,
                unlock_time: None,
            }],
            false,
            999,
            noop_gate,
        )
        .expect("relock commit should succeed");

        assert_eq!(report.unlocked_after, 1);
        assert_eq!(report.applied_relocks, 1);

        // Verify data AND pendingbits AND AchievementTimes no longer contain bit 1
        let doc = parse_binary(&std::fs::read(&sb).unwrap()).unwrap();
        let b0 = doc.root.get_dict("cache").unwrap().get_dict("0").unwrap();
        assert_eq!(b0.get("data"), Some(&KvValue::Int32(0b01)));
        assert_eq!(b0.get("pendingbits"), Some(&KvValue::Int32(0b00)));
        assert_eq!(b0.get_dict("AchievementTimes").unwrap().get("1"), None);

        // Backup bin & json must also drop achievement 1 so AetherDLL's monotone guard won't revert it!
        let backup_parsed =
            parse_user_stats_bin(&std::fs::read(&bb).unwrap(), &BTreeSet::from([0])).unwrap();
        assert_eq!(backup_parsed.unlocked.len(), 1);
        assert!(backup_parsed.unlocked.contains_key(&0));
        assert!(!backup_parsed.unlocked.contains_key(&1));

        let (json_unlocks, _) = parse_backup_json(&std::fs::read_to_string(&bj).unwrap()).unwrap();
        assert_eq!(json_unlocks.len(), 1);
        assert!(!json_unlocks.contains_key(&1));
    }

    #[test]
    fn cold_creates_all_three_files_when_user_cache_never_existed() {
        let tmp = TempDir::new().unwrap();
        let steam = tmp.path().join("steam");
        let data = tmp.path().join("AetherData");
        let acct = 55u32;
        let app = 2124490u32;

        write_test_schema(&steam, app);
        write_loginusers(&steam, acct);

        let report = commit_achievement_changes_blocking(
            &steam,
            &data,
            app,
            acct,
            &[AchievementChange {
                id: 0,
                unlocked: true,
                unlock_time: None,
            }],
            false,
            1_760_123_456,
            noop_gate,
        )
        .expect("cold commit should succeed");

        assert_eq!(report.unlocked_after, 1);
        assert!(steam_user_bin_path(&steam, acct, app).is_file());
        assert!(backup_user_bin_path(&data, acct, app).is_file());
        assert!(backup_user_json_path(&data, acct, app).is_file());
    }

    #[test]
    fn refuses_commit_when_steam_is_running_without_kill_consent() {
        let tmp = TempDir::new().unwrap();
        let steam = tmp.path().join("steam");
        let data = tmp.path().join("AetherData");
        write_test_schema(&steam, 2124490);
        write_loginusers(&steam, 77);

        let err = commit_achievement_changes_blocking(
            &steam,
            &data,
            2124490,
            77,
            &[AchievementChange {
                id: 0,
                unlocked: true,
                unlock_time: None,
            }],
            false,
            500,
            refuse_gate,
        )
        .expect_err("must refuse when Steam running without consent");
        assert!(err.starts_with("STEAM_RUNNING:"));
    }

    #[test]
    fn inspect_reports_lua_managed_state_and_icons() {
        let tmp = TempDir::new().unwrap();
        let steam = tmp.path().join("steam");
        let data = tmp.path().join("AetherData");
        write_test_schema(&steam, 2124490);
        write_loginusers(&steam, 77);

        assert!(now_unix_seconds() > 0);
        let snap = inspect_game_achievements(&steam, &data, 2124490, None).unwrap();
        assert!(!snap.lua_managed);
        assert_eq!(snap.account_id, 77);
        assert_eq!(snap.total_count, 2);
        assert!(snap.achievements[0]
            .icon_url
            .as_deref()
            .unwrap()
            .ends_with("/2124490/one.jpg"));
    }

    #[test]
    fn rolls_back_backup_files_if_steam_cache_write_fails() {
        let tmp = TempDir::new().unwrap();
        let steam = tmp.path().join("steam");
        let data = tmp.path().join("AetherData");
        let acct = 88u32;
        let app = 2124490u32;
        write_test_schema(&steam, app);
        write_loginusers(&steam, acct);

        let sb = steam_user_bin_path(&steam, acct, app);
        let bb = backup_user_bin_path(&data, acct, app);
        let bj = backup_user_json_path(&data, acct, app);
        std::fs::create_dir_all(sb.parent().unwrap()).unwrap();
        std::fs::create_dir_all(bb.parent().unwrap()).unwrap();
        let orig_bin = make_existing_bin(0, 0, vec![]);
        let orig_json = "{\"original\":true}\n";
        std::fs::write(&sb, &orig_bin).unwrap();
        std::fs::write(&bb, &orig_bin).unwrap();
        std::fs::write(&bj, orig_json).unwrap();

        let err = commit_achievement_changes_with_writer(
            &steam,
            &data,
            app,
            acct,
            &[AchievementChange {
                id: 0,
                unlocked: true,
                unlock_time: Some(12345),
            }],
            false,
            12345,
            noop_gate,
            |path, bytes| {
                if path == sb {
                    Err("simulated steam bin write error".to_string())
                } else {
                    write_atomic(path, bytes)
                }
            },
        )
        .expect_err("must fail when Steam bin write fails");
        assert!(err.contains("simulated steam bin write error"));
        assert_eq!(std::fs::read(&sb).unwrap(), orig_bin);
        assert_eq!(std::fs::read(&bb).unwrap(), orig_bin);
        assert_eq!(std::fs::read_to_string(&bj).unwrap(), orig_json);
    }
}
