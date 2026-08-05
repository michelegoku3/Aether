//! Batched Steam store metadata via `IStoreBrowseService/GetItems/v1`.
//!
//! Ported from SFF's `_fetch_steam_platforms`, narrowed to what the store search
//! needs: app *kind*, `parent_appid` and the "delisted blank" signal.
//!
//! Why GetItems and not `appdetails`: appdetails enforces a strict
//! ~200-requests/5-min per-IP rate limit (HTTP 429) and rejects multi-appid
//! queries (verified: HTTP 400). GetItems batches ~50 appids per request with
//! no visible rate limit, so one call covers a whole search result page.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use crate::providers::http;

const GET_ITEMS_URL: &str = "https://api.steampowered.com/IStoreBrowseService/GetItems/v1";
const GET_ITEMS_TIMEOUT_SECONDS: u64 = 8;

/// 50 per call is the conservative SFF default: Steam accepts more but the URL
/// grows fast (payload is JSON, URL-encoded as a query parameter).
const CHUNK_SIZE: usize = 48;

/// After this many consecutive chunk failures the remaining ids are marked
/// "unknown" so a transient Steam outage cannot stall the search worker.
const MAX_CONSECUTIVE_FAILURES: usize = 2;

/// Structural metadata for one store item. All fields default to the
/// "unknown" state, and callers must treat unknown as *keep the row* —
/// the filter only drops rows Steam explicitly tagged as DLC-ish.
#[derive(Debug, Clone, Default)]
pub struct StoreItemMeta {
    /// Lowercase type string ("game", "dlc", "demo", "mod", "rerelease",
    /// "tool", "video", "music", "advertising"); "" when GetItems had no body.
    pub kind: String,
    /// Set only for DLC of another app; None for base games and demos.
    pub parent_appid: Option<u32>,
    /// True when Steam returned a row with no name and no type: it strips all
    /// public metadata from fully removed (delisted) DLC content, while real
    /// delisted *games* keep name + type=game (SFF-verified on GTA SA classic,
    /// Dark Souls PTDE, Resident Evil HD).
    pub delisted_blank: bool,
}

/// The SFF structural DLC rule set, three signals with no name matching:
///
/// 1. `parent_appid` set → DLC of another app. Re-releases hang off the base
///    appid the same way but ship as standalone games; Steam tags them
///    `type: 14` → we keep "rerelease" rows and drop everything else.
/// 2. `delisted_blank` → removed-from-store DLC content (see field docs).
/// 3. Belt-and-suspenders type drop: tool/video/music/advertising/dlc kinds
///    without a parent (edge cases GetItems reports with no parent set).
pub fn is_dlc_like(meta: &StoreItemMeta) -> bool {
    if meta.delisted_blank {
        return true;
    }
    if meta.parent_appid.is_some() && meta.kind != "rerelease" {
        return true;
    }
    if !meta.kind.is_empty()
        && !matches!(meta.kind.as_str(), "game" | "demo" | "mod" | "rerelease")
    {
        return true;
    }
    false
}

/// Integer type codes used by GetItems, mapped to lowercase strings so the
/// filter logic reads like SFF's. Unknown codes stringified for debuggability.
fn map_type_code(code: i64) -> String {
    match code {
        0 => "game".to_string(),
        2 | 4 => "dlc".to_string(),
        3 => "demo".to_string(),
        5 => "advertising".to_string(),
        6 => "mod".to_string(),
        7 => "tool".to_string(),
        9..=12 => "video".to_string(),
        13 => "music".to_string(),
        14 => "rerelease".to_string(),
        15 => "video".to_string(),
        other => other.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct GetItemsResponse {
    response: Option<GetItemsBody>,
}

#[derive(Debug, Deserialize)]
struct GetItemsBody {
    store_items: Option<Vec<GetStoreItem>>,
}

#[derive(Debug, Deserialize)]
struct GetStoreItem {
    appid: Option<u32>,
    name: Option<String>,
    #[serde(rename = "type")]
    type_code: Option<i64>,
    related_items: Option<RelatedItems>,
}

#[derive(Debug, Deserialize)]
struct RelatedItems {
    parent_appid: Option<u32>,
}

/// Process-lifetime cache: repeat searches for the same ids cost zero network
/// calls (mirrors SFF's `_STEAM_PLATFORM_CACHE`).
fn meta_cache() -> &'static Mutex<HashMap<u32, StoreItemMeta>> {
    static CACHE: OnceLock<Mutex<HashMap<u32, StoreItemMeta>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Fetch structural metadata for `app_ids`, batched in chunks.
///
/// Ids with no metadata available map to `StoreItemMeta::default()` ("unknown"
/// = keep), so the caller never loses rows because Steam refused to answer.
pub async fn fetch_store_items(app_ids: Vec<u32>) -> HashMap<u32, StoreItemMeta> {
    let mut out: HashMap<u32, StoreItemMeta> = HashMap::new();
    let mut pending: Vec<u32> = Vec::new();

    for app_id in app_ids {
        if app_id == 0 {
            continue;
        }
        let cached = meta_cache()
            .lock()
            .ok()
            .and_then(|cache| cache.get(&app_id).cloned());
        match cached {
            Some(meta) => {
                out.insert(app_id, meta);
            }
            None => pending.push(app_id),
        }
    }

    pending.sort_unstable();
    pending.dedup();

    if pending.is_empty() {
        return out;
    }

    let client = http::build_client(GET_ITEMS_TIMEOUT_SECONDS);
    let mut consecutive_failures = 0usize;

    for chunk in pending.chunks(CHUNK_SIZE) {
        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            mark_unknown(chunk, &mut out);
            continue;
        }

        match fetch_chunk(&client, chunk).await {
            Ok(metas) => {
                consecutive_failures = 0;
                if let Ok(mut cache) = meta_cache().lock() {
                    for (app_id, meta) in &metas {
                        cache.insert(*app_id, meta.clone());
                    }
                }
                out.extend(metas);
            }
            Err(e) => {
                consecutive_failures += 1;
                eprintln!(
                    "[SteamItems] GetItems failed for chunk starting at {}: {}",
                    chunk[0], e
                );
                mark_unknown(chunk, &mut out);
            }
        }
    }

    out
}

fn mark_unknown(chunk: &[u32], out: &mut HashMap<u32, StoreItemMeta>) {
    for app_id in chunk {
        out.entry(*app_id).or_default();
    }
}

async fn fetch_chunk(
    client: &reqwest::Client,
    chunk: &[u32],
) -> Result<HashMap<u32, StoreItemMeta>, String> {
    let payload = serde_json::json!({
        "ids": chunk.iter().map(|id| serde_json::json!({ "appid": id })).collect::<Vec<_>>(),
        "context": { "language": "english", "country_code": "US" },
        "data_request": {
            "include_assets": false,
            "include_platforms": true,
            "include_basic_info": false,
            "include_release": false,
        }
    });

    let response = client
        .get(GET_ITEMS_URL)
        .query(&[("input_json", payload.to_string())])
        .send()
        .await
        .map_err(|e| format!("GetItems request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GetItems returned HTTP {}", response.status()));
    }

    let data = response
        .json::<GetItemsResponse>()
        .await
        .map_err(|e| format!("Failed to parse GetItems response: {}", e))?;

    let items = data
        .response
        .and_then(|body| body.store_items)
        .unwrap_or_default();

    let mut out = HashMap::new();
    let mut seen: Vec<u32> = Vec::new();
    for item in items {
        let Some(app_id) = item.appid else { continue };
        seen.push(app_id);

        let name_empty = item
            .name
            .as_ref()
            .map(|n| n.trim().is_empty())
            .unwrap_or(true);

        let parent_appid = item
            .related_items
            .and_then(|related| related.parent_appid)
            .filter(|parent| *parent > 0);

        out.insert(
            app_id,
            StoreItemMeta {
                kind: item.type_code.map(map_type_code).unwrap_or_default(),
                parent_appid,
                // Steam strips name + type from fully delisted DLC entries.
                delisted_blank: name_empty && item.type_code.is_none(),
            },
        );
    }

    // Anything GetItems silently dropped gets the "unknown" sentinel.
    for app_id in chunk {
        out.entry(*app_id).or_default();
    }

    Ok(out)
}
