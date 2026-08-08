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

/// Steam content-descriptor ids that mark sexually explicit material:
/// 3 = "Nudity or Sexual Content", 4 = "Adult Only Sexual Content".
/// Verified empirically: Witcher 3 → [1,5], Cyberpunk → [1,2,5],
/// HuniePop/Mirror → [1,3,4,5]. Ids 1/2/5 are violence/mature-language
/// markers and must NOT flag a game as NSFW (SFF uses {1,2,3,4}, which also
/// blocks violent-but-not-sexual titles — deliberately not replicated here).
const NSFW_CONTENT_DESCRIPTOR_IDS: [u32; 2] = [3, 4];

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
    /// True when Steam attached a sexual content descriptor (3 or 4).
    /// Combined with `looks_nsfw_by_name` by `is_nsfw` because some adult
    /// titles ship without descriptors (and delisted rows carry no metadata).
    pub is_nsfw: bool,
    /// True when Steam flags the item as `unlisted`: removed from the store
    /// catalog but its app page/hub still exists (classic delisted games:
    /// GTA SA classic, Dark Souls PTDE, Spec Ops: The Line — verified
    /// empirically against GetItems; F2P titles never carry the flag).
    pub is_delisted: bool,
    /// Unix seconds of the Steam release date (`release.steam_release_date`),
    /// None when Steam does not report one. Used for newest-first ordering.
    pub release_date_unix: Option<i64>,
    /// Original release date when Steam reports it separately from the Steam
    /// release. Useful for the Info modal; not used for filtering.
    pub original_release_date_unix: Option<i64>,
    pub visible: Option<bool>,
    pub store_url_path: Option<String>,
    pub platforms: StoreItemPlatforms,
    pub categories: StoreItemCategories,
    pub best_purchase_option: Option<StoreItemPurchaseOption>,
    pub content_descriptor_ids: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct StoreItemPlatforms {
    pub windows: Option<bool>,
    pub mac: Option<bool>,
    pub linux: Option<bool>,
    pub steam_deck_compat_category: Option<u32>,
    pub steam_os_compat_category: Option<u32>,
    pub steam_machine_compat_category: Option<u32>,
    pub has_vr_support: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct StoreItemCategories {
    pub supported_player_category_ids: Vec<u32>,
    pub feature_category_ids: Vec<u32>,
    pub controller_category_ids: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct StoreItemPurchaseOption {
    pub formatted_final_price: Option<String>,
    pub final_price_in_cents: Option<String>,
    pub purchase_option_name: Option<String>,
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

/// Name-based NSFW heuristic (ported from SFF's `_NSFW_NAME_RE`, but applied
/// to whole tokens instead of raw substring so "Essex"-style names can't
/// false-positive). It is a safety net for adult titles without descriptors.
pub fn looks_nsfw_by_name(name: &str) -> bool {
    name.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|token| matches!(token, "hentai" | "futanari" | "furry" | "sex" | "sexy" | "porn"))
}

/// The single NSFW classification entry point: descriptor first, name as
/// safety net. Keeping both signals behind one function means callers never
/// mix the rules by hand (DRY).
pub fn is_nsfw(meta: &StoreItemMeta, name: &str) -> bool {
    meta.is_nsfw || looks_nsfw_by_name(name)
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
    content_descriptorids: Option<Vec<u32>>,
    unlisted: Option<bool>,
    release: Option<ReleaseInfo>,
    visible: Option<bool>,
    store_url_path: Option<String>,
    platforms: Option<GetItemPlatforms>,
    categories: Option<GetItemCategories>,
    best_purchase_option: Option<GetItemPurchaseOption>,
}

#[derive(Debug, Deserialize)]
struct ReleaseInfo {
    steam_release_date: Option<i64>,
    original_release_date: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RelatedItems {
    parent_appid: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GetItemPlatforms {
    windows: Option<bool>,
    mac: Option<bool>,
    linux: Option<bool>,
    steam_deck_compat_category: Option<u32>,
    steam_os_compat_category: Option<u32>,
    steam_machine_compat_category: Option<u32>,
    vr_support: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GetItemCategories {
    supported_player_categoryids: Option<Vec<u32>>,
    feature_categoryids: Option<Vec<u32>>,
    controller_categoryids: Option<Vec<u32>>,
}

#[derive(Debug, Deserialize)]
struct GetItemPurchaseOption {
    formatted_final_price: Option<String>,
    final_price_in_cents: Option<String>,
    purchase_option_name: Option<String>,
}

/// Process-lifetime HTTP client: building a reqwest client per call means a
/// fresh TCP+TLS handshake to api.steampowered.com on every search, which was
/// the main latency source of the metadata batch. `Client` is an Arc inside,
/// so cloning the shared one is free and keeps connection pooling alive.
fn get_items_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| http::build_client(GET_ITEMS_TIMEOUT_SECONDS))
        .clone()
}

/// Process-lifetime cache: repeat searches for the same ids cost zero network
/// calls (mirrors SFF's `_STEAM_PLATFORM_CACHE`).
fn meta_cache() -> &'static Mutex<HashMap<String, StoreItemMeta>> {
    static CACHE: OnceLock<Mutex<HashMap<String, StoreItemMeta>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn meta_cache_key(country_code: &str, app_id: u32) -> String {
    format!("{}:{}", country_code.trim().to_uppercase(), app_id)
}

/// Fetch structural metadata for `app_ids`, batched in chunks.
///
/// Ids with no metadata available map to `StoreItemMeta::default()` ("unknown"
/// = keep), so the caller never loses rows because Steam refused to answer.
pub async fn fetch_store_items(app_ids: Vec<u32>) -> HashMap<u32, StoreItemMeta> {
    fetch_store_items_for_country(app_ids, "US").await
}

pub async fn fetch_store_items_for_country(app_ids: Vec<u32>, country_code: &str) -> HashMap<u32, StoreItemMeta> {
    let mut out: HashMap<u32, StoreItemMeta> = HashMap::new();
    let mut pending: Vec<u32> = Vec::new();

    for app_id in app_ids {
        if app_id == 0 {
            continue;
        }
        let cache_key = meta_cache_key(country_code, app_id);
        let cached = meta_cache()
            .lock()
            .ok()
            .and_then(|cache| cache.get(&cache_key).cloned());
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

    let client = get_items_client();
    let mut consecutive_failures = 0usize;

    for chunk in pending.chunks(CHUNK_SIZE) {
        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            mark_unknown(chunk, &mut out);
            continue;
        }

        match fetch_chunk(&client, chunk, country_code).await {
            Ok(metas) => {
                consecutive_failures = 0;
                if let Ok(mut cache) = meta_cache().lock() {
                    for (app_id, meta) in &metas {
                        cache.insert(meta_cache_key(country_code, *app_id), meta.clone());
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
    country_code: &str,
) -> Result<HashMap<u32, StoreItemMeta>, String> {
    let payload = serde_json::json!({
        "ids": chunk.iter().map(|id| serde_json::json!({ "appid": id })).collect::<Vec<_>>(),
        "context": { "language": "english", "country_code": country_code.trim().to_uppercase() },
        "data_request": {
            "include_assets": false,
            "include_platforms": true,
            "include_basic_info": false,
            "include_release": true,
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

        let content_descriptor_ids = item.content_descriptorids.clone().unwrap_or_default();
        let is_nsfw = content_descriptor_ids
            .iter()
            .any(|id| NSFW_CONTENT_DESCRIPTOR_IDS.contains(id));

        let release_date_unix = item
            .release
            .as_ref()
            .and_then(|release| release.steam_release_date)
            .filter(|date| *date > 0);
        let original_release_date_unix = item
            .release
            .as_ref()
            .and_then(|release| release.original_release_date)
            .filter(|date| *date > 0);

        let platforms = item.platforms.map(|platforms| StoreItemPlatforms {
            windows: platforms.windows,
            mac: platforms.mac,
            linux: platforms.linux,
            steam_deck_compat_category: platforms.steam_deck_compat_category,
            steam_os_compat_category: platforms.steam_os_compat_category,
            steam_machine_compat_category: platforms.steam_machine_compat_category,
            has_vr_support: platforms.vr_support.map(|value| {
                if let Some(object) = value.as_object() {
                    !object.is_empty()
                } else {
                    !value.is_null()
                }
            }),
        }).unwrap_or_default();

        let categories = item.categories.map(|categories| StoreItemCategories {
            supported_player_category_ids: categories.supported_player_categoryids.unwrap_or_default(),
            feature_category_ids: categories.feature_categoryids.unwrap_or_default(),
            controller_category_ids: categories.controller_categoryids.unwrap_or_default(),
        }).unwrap_or_default();

        let best_purchase_option = item.best_purchase_option.map(|option| StoreItemPurchaseOption {
            formatted_final_price: option.formatted_final_price,
            final_price_in_cents: option.final_price_in_cents,
            purchase_option_name: option.purchase_option_name,
        });

        out.insert(
            app_id,
            StoreItemMeta {
                kind: item.type_code.map(map_type_code).unwrap_or_default(),
                parent_appid,
                // Steam strips name + type from fully delisted DLC entries.
                delisted_blank: name_empty && item.type_code.is_none(),
                is_nsfw,
                is_delisted: item.unlisted.unwrap_or(false),
                release_date_unix,
                original_release_date_unix,
                visible: item.visible,
                store_url_path: item.store_url_path,
                platforms,
                categories,
                best_purchase_option,
                content_descriptor_ids,
            },
        );
    }

    // Anything GetItems silently dropped gets the "unknown" sentinel.
    for app_id in chunk {
        out.entry(*app_id).or_default();
    }

    Ok(out)
}
