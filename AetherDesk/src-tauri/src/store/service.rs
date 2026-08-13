use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// Overall wall-clock budget for the whole Hubcap availability lookup
/// (2 endpoints × up to 2 query variants). The two endpoint calls inside
/// each variant run in parallel, so this is not a per-request timeout:
/// it only guards against the minority of runs where Hubcap hangs.
const HUBCAP_SEARCH_BUDGET_MS: u64 = 6000;

/// Hard cap on rows sent to the GetItems metadata batch. The UI paginates 20
/// cards at a time, so 80 classified rows (4 pages) is generous, and it keeps
/// the batch at one chunk (~48 ids) in the common case instead of several
/// sequential ones for franchise-wide queries.
const MAX_CLASSIFIED_RESULTS: usize = 80;
use crate::game_info::model::{GameInfoPlatforms, GameInfoPrice, GameInfoStoreCategories};
use crate::steam::store::{SteamStore, SteamStoreItem};
use crate::steam::store_items;
use crate::store::{aliases, normalize};
use crate::providers::hubcap::HubcapClient;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UnifiedStoreGame {
    pub id: u32,
    pub name: String,
    #[serde(rename = "appId")]
    pub app_id: String,
    pub has_manifest: bool,
    pub has_denuvo: bool,
    /// True when Steam content descriptors (or the name heuristic) mark the
    /// title as sexually explicit. The UI uses it for the pink card border;
    /// it stays `false` on rows Steam cannot classify (unknown metadata).
    /// `#[serde(default)]` keeps pre-NSFW search-cache entries deserializable.
    #[serde(default)]
    pub has_nsfw: bool,
    /// True when Steam flags the item as `unlisted` (delisted). The UI uses
    /// it for the white card border. `#[serde(default)]` keeps older
    /// search-cache entries deserializable.
    #[serde(default)]
    pub has_delisted: bool,
    #[serde(rename = "imageUrl")]
    pub image_url: String,
    #[serde(default, rename = "heroImageUrl")]
    pub hero_image_url: String,
    #[serde(default)]
    pub store_kind: String,
    #[serde(default)]
    pub release_date_unix: Option<i64>,
    #[serde(default)]
    pub original_release_date_unix: Option<i64>,
    #[serde(default)]
    pub store_url_path: Option<String>,
    #[serde(default)]
    pub price: Option<GameInfoPrice>,
    #[serde(default)]
    pub metascore: Option<String>,
    #[serde(default)]
    pub controller_support: Option<String>,
    #[serde(default)]
    pub platforms: Option<GameInfoPlatforms>,
    #[serde(default)]
    pub store_categories: Option<GameInfoStoreCategories>,
    #[serde(default)]
    pub content_descriptor_ids: Vec<u32>,
}

pub struct StoreService {
    steam_store: SteamStore,
}

impl StoreService {
    pub fn new() -> Self {
        Self {
            steam_store: SteamStore::new(),
        }
    }

    /// Normalizes strings for high-fidelity comparison.
    /// Delegates to the shared `store::normalize` module so scoring, filtering
    /// and Hubcap sanitization share a single implementation (DRY).
    pub fn normalize_string(&self, s: &str) -> String {
        normalize::normalize_string(s)
    }

    /// Exact / prefix / substring, then **per-token** Damerau-Levenshtein.
    /// Whole-title edit distance cannot match `witchr 3` to a long official name.
    pub fn calculate_relevance_score(&self, query: &str, name: &str) -> usize {
        normalize::relevance_score(query, name)
    }

    /// Collect the merged Hubcap availability set for a query.
    ///
    /// Query variants:
    ///   1. `aliases::primary_variants` (original + first alias expansion).
    ///   2. For each variant, a *sanitized* form where punctuation/symbols are
    ///      replaced by spaces (e.g. "Take Me To The Dungeon!!" → "Take Me To The Dungeon").
    ///      Hubcap's substring matcher treats `!` literally and can 400-soft-fail;
    ///      the sanitized variant guarantees a punctuation-insensitive second shot
    ///      without requiring server-side changes.
    ///
    /// Deduplicated case-insensitively, capped to 4 parallel `search_all` calls
    /// (each itself fans out to `/library` + `/search`). Bounded by
    /// `HUBCAP_SEARCH_BUDGET_MS`; any failure degrades to empty so Steam still renders.
    async fn collect_hubcap_hits(
        &self,
        query: &str,
        hubcap_client: Option<&HubcapClient>,
    ) -> Vec<crate::providers::hubcap::HubcapGameItem> {
        let Some(client) = hubcap_client else {
            return Vec::new();
        };

        // Build deduplicated Hubcap query set: alias expansions + sanitized punctuation.
        let base_variants = aliases::primary_variants(query);
        let mut all_queries: Vec<String> = Vec::new();
        let mut seen_lower: HashSet<String> = HashSet::new();
        for base in &base_variants {
            for candidate in [
                base.clone(),
                normalize::sanitize_query_for_hubcap(base).unwrap_or_default(),
            ] {
                if candidate.is_empty() {
                    continue;
                }
                let lower = candidate.to_lowercase();
                if seen_lower.insert(lower) {
                    all_queries.push(candidate);
                }
            }
        }
        // Hard cap to 4 to bound network fan-out (2 alias * 2 sanitized).
        if all_queries.len() > 4 {
            all_queries.truncate(4);
        }

        if all_queries.is_empty() {
            return Vec::new();
        }

        // Clone client + owned query strings so each async branch owns its data
        // and no temporary is freed while borrowed.
        let client_for_lookup = client.clone();
        let queries_for_lookup = all_queries.clone();
        let lookup = async move {
            // Parallelize up to 4 `search_all` calls. Match on arity to avoid
            // pulling in `futures` crate; HubcapClient is Clone so we can move
            // a clone into each branch.
            let batches: Vec<Vec<crate::providers::hubcap::HubcapGameItem>> = match queries_for_lookup.len() {
                4 => {
                    let q0 = queries_for_lookup[0].clone();
                    let q1 = queries_for_lookup[1].clone();
                    let q2 = queries_for_lookup[2].clone();
                    let q3 = queries_for_lookup[3].clone();
                    let c0 = client_for_lookup.clone();
                    let c1 = client_for_lookup.clone();
                    let c2 = client_for_lookup.clone();
                    let c3 = client_for_lookup.clone();
                    let (a, b, c, d) = tokio::join!(
                        c0.search_all(&q0),
                        c1.search_all(&q1),
                        c2.search_all(&q2),
                        c3.search_all(&q3),
                    );
                    vec![a, b, c, d]
                }
                3 => {
                    let q0 = queries_for_lookup[0].clone();
                    let q1 = queries_for_lookup[1].clone();
                    let q2 = queries_for_lookup[2].clone();
                    let c0 = client_for_lookup.clone();
                    let c1 = client_for_lookup.clone();
                    let c2 = client_for_lookup.clone();
                    let (a, b, c) = tokio::join!(
                        c0.search_all(&q0),
                        c1.search_all(&q1),
                        c2.search_all(&q2),
                    );
                    vec![a, b, c]
                }
                2 => {
                    let q0 = queries_for_lookup[0].clone();
                    let q1 = queries_for_lookup[1].clone();
                    let c0 = client_for_lookup.clone();
                    let c1 = client_for_lookup.clone();
                    let (a, b) = tokio::join!(
                        c0.search_all(&q0),
                        c1.search_all(&q1),
                    );
                    vec![a, b]
                }
                1 => {
                    let q0 = queries_for_lookup[0].clone();
                    vec![client_for_lookup.clone().search_all(&q0).await]
                },
                _ => Vec::new(),
            };

            let mut merged: Vec<crate::providers::hubcap::HubcapGameItem> = Vec::new();
            let mut seen_ids = HashSet::new();
            for batch in batches {
                for item in batch {
                    if seen_ids.insert(item.app_id) {
                        merged.push(item);
                    }
                }
            }
            merged
        };

        match tokio::time::timeout(Duration::from_millis(HUBCAP_SEARCH_BUDGET_MS), lookup).await {
            Ok(games) => {
                eprintln!(
                    "[Hubcap] Search for '{}' ({} variant(s), {} queries) returned {} games",
                    query,
                    base_variants.len(),
                    all_queries.len(),
                    games.len()
                );
                games
            }
            Err(_) => {
                eprintln!("[Hubcap] Search timeout for '{}' ({}ms)", query, HUBCAP_SEARCH_BUDGET_MS);
                Vec::new()
            }
        }
    }


    /// Store front page: **Steam-only by design**.
    ///
    /// The store front must never touch Hubcap: per-item `/status` checks
    /// fanned out to 20–40 requests in a few seconds during normal browsing
    /// (2 pages preloaded on startup), which tripped Hubcap's per-IP rate
    /// limit almost immediately. Rows therefore carry no availability badge;
    /// the search flow remains the single place where Hubcap contributes the
    /// `has_manifest` flag.
    pub async fn trending_store(
        &self,
        store_front_filter: &str,
        start: usize,
        count: usize,
        show_store_dlcs: bool,
        show_store_nsfw: bool,
        show_store_delisted: bool,
        steam_country_code: &str,
    ) -> Result<Vec<UnifiedStoreGame>, String> {
        let steam_items = self
            .steam_store
            .store_front_for_country(store_front_filter, start, count, steam_country_code)
            .await?;

        if steam_items.is_empty() {
            return Ok(Vec::new());
        }

        let mut seen_ids = HashSet::new();
        let mut unified_list: Vec<UnifiedStoreGame> = Vec::new();
        for item in steam_items {
            if !seen_ids.insert(item.id) {
                continue;
            }
            unified_list.push(UnifiedStoreGame {
                id: item.id,
                name: item.name.clone(),
                app_id: item.id.to_string(),
                // Steam-only store front: availability is unknown here by design
                // (see the method doc — Hubcap status fan-out caused rate limits).
                has_manifest: false,
                has_denuvo: false,
                has_nsfw: false,
                has_delisted: false,
                image_url: item.image_url.clone(),
                hero_image_url: String::new(),
                store_kind: item.item_type.clone().unwrap_or_default(),
                release_date_unix: None,
                original_release_date_unix: None,
                store_url_path: None,
                price: price_from_store_item(&item),
                metascore: metascore_from_store_item(&item),
                controller_support: item.controller_support.clone(),
                platforms: platforms_from_store_item(&item),
                store_categories: None,
                content_descriptor_ids: Vec::new(),
            });
        }

        let meta_map = if !unified_list.is_empty() {
            let ids: Vec<u32> = unified_list.iter().map(|game| game.id).collect();
            store_items::fetch_store_items_for_country(ids, steam_country_code).await
        } else {
            HashMap::new()
        };

        for game in unified_list.iter_mut() {
            let meta = meta_map.get(&game.id).cloned().unwrap_or_default();
            game.has_nsfw = store_items::is_nsfw(&meta, &game.name);
            game.has_delisted = meta.is_delisted;
            if !meta.kind.is_empty() {
                game.store_kind = meta.kind.clone();
            }
            game.release_date_unix = meta.release_date_unix.or(game.release_date_unix);
            game.original_release_date_unix = meta.original_release_date_unix.or(game.original_release_date_unix);
            game.store_url_path = meta.store_url_path.clone().or_else(|| game.store_url_path.clone());
            if let Some(url) = meta.library_capsule_url.as_ref().filter(|url| !url.trim().is_empty()) {
                game.image_url = url.clone();
            }
            if let Some(url) = meta.hero_image_url.as_ref().filter(|url| !url.trim().is_empty()) {
                game.hero_image_url = url.clone();
            }
            game.platforms = Some(platforms_from_store_meta(&meta));
            game.store_categories = Some(categories_from_store_meta(&meta));
            game.content_descriptor_ids = meta.content_descriptor_ids.clone();
            if game.price.is_none() {
                game.price = price_from_store_meta(&meta);
            }
        }

        unified_list.retain(|game| {
            let meta = meta_map.get(&game.id).cloned().unwrap_or_default();
            if !show_store_dlcs && store_items::is_dlc_like(&meta) {
                return false;
            }
            if !show_store_nsfw && store_items::is_nsfw(&meta, &game.name) {
                return false;
            }
            if !show_store_delisted && meta.is_delisted {
                return false;
            }
            true
        });

        Ok(unified_list)
    }

    /// Queries Steam and Hubcap in parallel, merges, and applies high-fidelity fuzzy/relevance sorting.
    ///
    /// Source roles (mirroring the simple part of SFF's store search):
    ///   * Steam `storesearch` is the *catalog*: names and cover images.
    ///   * Hubcap is the *availability authority*: for every query variant
    ///     (original + first alias expansion) both `/library` and `/search`
    ///     are queried in parallel and merged by app id.
    /// Neither source may take the other down: a Steam outage still yields the
    /// Hubcap-only list, a Hubcap outage still yields the plain Steam catalog.
    pub async fn search_store(&self, query: &str, hubcap_client: Option<HubcapClient>, show_store_dlcs: bool, show_store_nsfw: bool, show_store_delisted: bool, steam_country_code: &str) -> Result<Vec<UnifiedStoreGame>, String> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 1. Fetch from Steam and Hubcap in parallel.
        // Steam: try both raw and sanitized query (punctuation-insensitive) in parallel,
        // mirroring Hubcap's sanitized fallback. Storesearch handles `!!` today, but
        // sanitization guarantees coverage if Steam's matcher ever treats symbols literally
        // or if Hubcap's name copy differs in punctuation.
        let steam_queries: Vec<String> = {
            let mut v: Vec<String> = Vec::new();
            let mut seen = HashSet::new();
            let push = |list: &mut Vec<String>, seen: &mut HashSet<String>, value: String| {
                let key = value.trim().to_lowercase();
                if key.is_empty() || !seen.insert(key) {
                    return;
                }
                list.push(value);
            };
            for variant in aliases::primary_variants(query) {
                push(&mut v, &mut seen, variant.clone());
                if let Some(san) = normalize::sanitize_query_for_hubcap(&variant) {
                    push(&mut v, &mut seen, san);
                }
            }
            if v.len() > 3 {
                v.truncate(3);
            }
            v
        };
        let steam_store_clone = self.steam_store.clone();
        let steam_queries_for_fut = steam_queries.clone();
        let steam_future = async move {
            let mut merged: Vec<crate::steam::store::SteamStoreItem> = Vec::new();
            let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
            // Fire up to 2 Steam searches in parallel via JoinSet-style match.
            let batches: Vec<Result<Vec<crate::steam::store::SteamStoreItem>, String>> = match steam_queries_for_fut.len() {
                3 => {
                    let q0 = steam_queries_for_fut[0].clone();
                    let q1 = steam_queries_for_fut[1].clone();
                    let q2 = steam_queries_for_fut[2].clone();
                    let s0 = steam_store_clone.clone();
                    let s1 = steam_store_clone.clone();
                    let s2 = steam_store_clone.clone();
                    let (a, b, c) = tokio::join!(
                        s0.search_catalog_for_country(&q0, steam_country_code),
                        s1.search_catalog_for_country(&q1, steam_country_code),
                        s2.search_catalog_for_country(&q2, steam_country_code)
                    );
                    vec![a, b, c]
                }
                2 => {
                    let q0 = steam_queries_for_fut[0].clone();
                    let q1 = steam_queries_for_fut[1].clone();
                    let s0 = steam_store_clone.clone();
                    let s1 = steam_store_clone.clone();
                    let (a, b) = tokio::join!(
                        s0.search_catalog_for_country(&q0, steam_country_code),
                        s1.search_catalog_for_country(&q1, steam_country_code)
                    );
                    vec![a, b]
                }
                1 => {
                    let q0 = steam_queries_for_fut[0].clone();
                    vec![steam_store_clone.search_catalog_for_country(&q0, steam_country_code).await]
                },
                _ => Vec::new(),
            };
            for res in batches {
                match res {
                    Ok(items) => {
                        for it in items {
                            if seen.insert(it.id) {
                                merged.push(it);
                            }
                        }
                    }
                    Err(e) => eprintln!("[Store] Steam catalog chunk failed for '{}': {}", query, e),
                }
            }
            if merged.is_empty() {
                // Preserve the "empty means no results, not error" contract.
                Ok::<Vec<crate::steam::store::SteamStoreItem>, String>(Vec::new())
            } else {
                Ok(merged)
            }
        };

        let hubcap_future = self.collect_hubcap_hits(query, hubcap_client.as_ref());

        let (steam_res, hubcap_res) = tokio::join!(steam_future, hubcap_future);
        // Graceful degradation: if the Steam catalog request fails, do not
        // discard the Hubcap results — return the Hubcap-only list instead.
        let steam_items = match steam_res {
            Ok(items) => items,
            Err(e) => {
                eprintln!("[Store] Steam catalog failed for '{}': {}", query, e);
                Vec::new()
            }
        };

        // 2. Create a fast O(1) lookup set of available app IDs on Hubcap.
        // Exact-match fallback: if the textual Hubcap search missed a game that
        // Steam returned with an exact normalized name hit, verify it directly
        // via `has_manifest(app_id)`. This covers any future Hubcap substring
        // edge-cases (punctuation, 400 soft-fail, etc) with at most one extra
        // cheap HEAD-like request for the exact candidate — still fail-open.
        let mut available_ids: HashSet<u32> = hubcap_res.iter().map(|g| g.app_id).collect();
        // Only probe when a Hubcap client exists and we have at least one exact
        // Steam hit that isn't already marked available.
        if let Some(client) = hubcap_client.as_ref() {
            // Collect exact-score candidates (score 0) that lack a manifest flag.
            let mut exact_missing: Vec<u32> = Vec::new();
            for item in &steam_items {
                if available_ids.contains(&item.id) {
                    continue;
                }
                if self.calculate_relevance_score(query, &item.name) == 0 {
                    exact_missing.push(item.id);
                    if exact_missing.len() >= 3 {
                        break; // cap probe fan-out
                    }
                }
            }
            if !exact_missing.is_empty() {
                // Probe sequentially with a short per-probe timeout so we don't
                // blow the overall 6s budget. Hubcap `has_manifest` reuses the
                // same auth headers and 8s client timeout.
                for app_id in exact_missing {
                    // Wrap in a 4s timeout so a hanging Hubcap doesn't stall the search.
                    let probe = tokio::time::timeout(
                        Duration::from_millis(4000),
                        client.has_manifest(app_id),
                    )
                    .await;
                    match probe {
                        Ok(true) => {
                            eprintln!("[Hubcap] Exact fallback verified manifest for {}", app_id);
                            available_ids.insert(app_id);
                        }
                        Ok(false) => {
                            eprintln!("[Hubcap] Exact fallback: no manifest for {}", app_id);
                        }
                        Err(_) => {
                            eprintln!("[Hubcap] Exact fallback timeout for {}", app_id);
                        }
                    }
                }
            }
        }

        let mut unified_list = Vec::new();
        let mut added_ids = HashSet::new();

        // 3. Populate unified list with Steam search results and overlay manifest availability.
        // DRM/Denuvo enrichment is intentionally NOT done here: it is slower Steam appdetails
        // metadata and is fetched by the frontend after first results are already rendered.
        for item in steam_items {
            let has_manifest = available_ids.contains(&item.id);
            unified_list.push(UnifiedStoreGame {
                id: item.id,
                name: item.name.clone(),
                app_id: item.id.to_string(),
                has_manifest,
                has_denuvo: false,
                has_nsfw: false,
                has_delisted: false,
                image_url: item.image_url.clone(),
                hero_image_url: String::new(),
                store_kind: item.item_type.clone().unwrap_or_default(),
                release_date_unix: None,
                original_release_date_unix: None,
                store_url_path: None,
                price: price_from_store_item(&item),
                metascore: metascore_from_store_item(&item),
                controller_support: item.controller_support.clone(),
                platforms: platforms_from_store_item(&item),
                store_categories: None,
                content_descriptor_ids: Vec::new(),
            });
            added_ids.insert(item.id);
        }

        // 4. Fallback: Hubcap matches that are NOT in Steam results are appended
        // (they were never in a Steam catalog checkout, so they arrive untyped).
        // They are collected separately first so the DLC filter below can see the
        // *merged* list in one pass.
        let mut hubcap_extras = Vec::new();
        for hg in hubcap_res {
            if !added_ids.contains(&hg.app_id) {
                hubcap_extras.push(UnifiedStoreGame {
                    id: hg.app_id,
                    name: hg.name,
                    app_id: hg.app_id.to_string(),
                    has_manifest: true,
                    has_denuvo: false,
                    has_nsfw: false,
                    has_delisted: false,
                    image_url: String::new(),
                    hero_image_url: String::new(),
                    store_kind: String::new(),
                    release_date_unix: None,
                    original_release_date_unix: None,
                    store_url_path: None,
                    price: None,
                    metascore: None,
                    controller_support: None,
                    platforms: None,
                    store_categories: None,
                    content_descriptor_ids: Vec::new(),
                });
                added_ids.insert(hg.app_id);
            }
        }
        let steam_result_ids = added_ids.clone();
        unified_list.extend(hubcap_extras);

        // 4a. Cheap pre-filter + cap BEFORE the metadata batch.
        // A generic query ("call of duty") can drag in hundreds of Hubcap tail
        // rows; paying a GetItems chunk for each was the search slowdown. The
        // relevance scorer already decides what step 5 will keep, so rows it
        // would discard as garbage are dropped now, and the rest is capped to
        // what the UI can reasonably paginate through (keeps GetItems at one
        // batch chunk in the common case). Numeric (AppID) queries skip this.
        let is_numeric = query.trim().parse::<u32>().is_ok();
        let score_variants = aliases::expanded_queries(query);
        let best_score = |name: &str| -> usize {
            score_variants
                .iter()
                .map(|variant| self.calculate_relevance_score(variant, name))
                .min()
                .unwrap_or(10000)
        };
        if !is_numeric {
            unified_list.retain(|game| {
                steam_result_ids.contains(&game.id) || best_score(&game.name) < 10000
            });
            if unified_list.len() > MAX_CLASSIFIED_RESULTS {
                let mut keyed: Vec<(usize, UnifiedStoreGame)> = unified_list
                    .into_iter()
                    .map(|game| (best_score(&game.name), game))
                    .collect();
                keyed.sort_by_key(|(score, _)| *score);
                keyed.truncate(MAX_CLASSIFIED_RESULTS);
                unified_list = keyed.into_iter().map(|(_, game)| game).collect();
            }
        }

        // 4b. Structural classification + filtering over the WHOLE merged list.
        // One batched GetItems call (~50 ids per call, process-cached) provides
        // the classifiers (DLC/NSFW/delisted) AND the release dates for ordering
        // in one shot. "Unknown" metadata always keeps the row.
        let meta_map = if !unified_list.is_empty() {
            let all_ids: Vec<u32> = unified_list.iter().map(|game| game.id).collect();
            store_items::fetch_store_items_for_country(all_ids, steam_country_code).await
        } else {
            HashMap::new()
        };
        if !unified_list.is_empty() {

            // Tag every row first: the NSFW/delisted flags feed the UI's
            // marker borders, so rows that survive the filters must carry
            // them; hidden rows are tagged too (harmless) because tagging and
            // filtering share the same metadata pass.
            for game in unified_list.iter_mut() {
                let meta = meta_map.get(&game.id).cloned().unwrap_or_default();
                game.has_nsfw = store_items::is_nsfw(&meta, &game.name);
                game.has_delisted = meta.is_delisted;
                if !meta.kind.is_empty() {
                    game.store_kind = meta.kind.clone();
                }
                game.release_date_unix = meta.release_date_unix.or(game.release_date_unix);
                game.original_release_date_unix = meta.original_release_date_unix.or(game.original_release_date_unix);
            game.store_url_path = meta.store_url_path.clone().or_else(|| game.store_url_path.clone());
            if let Some(url) = meta.library_capsule_url.as_ref().filter(|url| !url.trim().is_empty()) {
                game.image_url = url.clone();
            }
            if let Some(url) = meta.hero_image_url.as_ref().filter(|url| !url.trim().is_empty()) {
                game.hero_image_url = url.clone();
            }
            game.platforms = Some(platforms_from_store_meta(&meta));
                game.store_categories = Some(categories_from_store_meta(&meta));
                game.content_descriptor_ids = meta.content_descriptor_ids.clone();
                if game.price.is_none() {
                    game.price = price_from_store_meta(&meta);
                }
            }

            let mut dlc_filtered = 0usize;
            let mut nsfw_filtered = 0usize;
            let mut delisted_filtered = 0usize;
            unified_list.retain(|game| {
                let meta = meta_map.get(&game.id).cloned().unwrap_or_default();
                if !show_store_dlcs && store_items::is_dlc_like(&meta) {
                    dlc_filtered += 1;
                    return false;
                }
                if !show_store_nsfw && store_items::is_nsfw(&meta, &game.name) {
                    nsfw_filtered += 1;
                    return false;
                }
                if !show_store_delisted && meta.is_delisted {
                    delisted_filtered += 1;
                    return false;
                }
                true
            });
            if dlc_filtered + nsfw_filtered + delisted_filtered > 0 {
                eprintln!(
                    "[Store] Filters for '{}': dropped {} DLC + {} NSFW + {} delisted row(s)",
                    query, dlc_filtered, nsfw_filtered, delisted_filtered
                );
            }
        }

        // 5. Clustered ordering: relevance TIER first, release date within
        // the tier. The scorer's numeric ranges define natural clusters —
        // exact (0), prefix (<100), substring (<1000), fuzzy (<10000) — and
        // sorting newest-first *inside* each tier keeps franchise members in
        // a contiguous, date-ordered block: "nba 2k" yields
        // [NBA 2K27, NBA 2K26, NBA 2K25, …] instead of being interleaved with
        // unrelated recent games. Step 4a already dropped score-10000 rows.
        if !is_numeric {
            let mut scored_items: Vec<(usize, UnifiedStoreGame)> = unified_list
                .into_iter()
                .map(|item| (best_score(&item.name), item))
                .collect();

            let tier_of = |score: usize| -> usize {
                match score {
                    0 => 0,
                    1..=99 => 1,
                    100..=999 => 2,
                    _ => 3,
                }
            };
            let release_of = |id: u32| -> i64 {
                meta_map
                    .get(&id)
                    .and_then(|meta| meta.release_date_unix)
                    .unwrap_or(0)
            };
            scored_items.sort_by(|a, b| {
                tier_of(a.0)
                    .cmp(&tier_of(b.0))
                    .then_with(|| release_of(b.1.id).cmp(&release_of(a.1.id)))
                    .then_with(|| a.0.cmp(&b.0))
            });

            Ok(scored_items.into_iter().map(|(_, item)| item).collect())
        } else {
            // If it is an App ID, exact or substring matches on digits are kept, no string sorting needed
            Ok(unified_list)
        }
    }
}

fn price_from_store_item(item: &SteamStoreItem) -> Option<GameInfoPrice> {
    item.price.as_ref().map(|price| GameInfoPrice {
        currency: price.currency.clone(),
        initial_cents: price.initial,
        final_cents: price.final_price,
        formatted_final: price.final_price.map(|final_price| format_price_fallback(final_price, price.currency.as_deref())),
        discount_percent: price.discount_percent,
    })
}

fn price_from_store_meta(meta: &store_items::StoreItemMeta) -> Option<GameInfoPrice> {
    meta.best_purchase_option.as_ref().map(|price| GameInfoPrice {
        currency: None,
        initial_cents: None,
        final_cents: price
            .final_price_in_cents
            .as_ref()
            .and_then(|value| value.parse::<i64>().ok()),
        formatted_final: price.formatted_final_price.clone(),
        discount_percent: None,
    })
}

fn format_price_fallback(amount_minor: i64, currency: Option<&str>) -> String {
    match currency.unwrap_or("EUR").to_uppercase().as_str() {
        "USD" => format!("${:.2}", amount_minor as f64 / 100.0),
        "JPY" => format!("¥{}", amount_minor),
        "EUR" => format!("€{:.2}", amount_minor as f64 / 100.0),
        other => format!("{} {:.2}", other, amount_minor as f64 / 100.0),
    }
}

fn metascore_from_store_item(item: &SteamStoreItem) -> Option<String> {
    item.metascore.as_ref().and_then(|value| match value {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn platforms_from_store_item(item: &SteamStoreItem) -> Option<GameInfoPlatforms> {
    item.platforms.as_ref().map(|platforms| GameInfoPlatforms {
        windows: platforms.windows,
        mac: platforms.mac,
        linux: platforms.linux,
        steam_deck_compat_category: None,
        steam_os_compat_category: None,
        steam_machine_compat_category: None,
        has_vr_support: None,
    })
}

fn platforms_from_store_meta(meta: &store_items::StoreItemMeta) -> GameInfoPlatforms {
    GameInfoPlatforms {
        windows: meta.platforms.windows,
        mac: meta.platforms.mac,
        linux: meta.platforms.linux,
        steam_deck_compat_category: meta.platforms.steam_deck_compat_category,
        steam_os_compat_category: meta.platforms.steam_os_compat_category,
        steam_machine_compat_category: meta.platforms.steam_machine_compat_category,
        has_vr_support: meta.platforms.has_vr_support,
    }
}

fn categories_from_store_meta(meta: &store_items::StoreItemMeta) -> GameInfoStoreCategories {
    GameInfoStoreCategories {
        supported_player_category_ids: meta.categories.supported_player_category_ids.clone(),
        feature_category_ids: meta.categories.feature_category_ids.clone(),
        controller_category_ids: meta.categories.controller_category_ids.clone(),
    }
}
