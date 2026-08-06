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
use crate::steam::store::SteamStore;
use crate::steam::store_items;
use crate::store::aliases;
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

    /// Normalizes strings for high-fidelity comparison (removes punctuation, Roman-numeral conversion, synonyms)
    pub fn normalize_string(&self, s: &str) -> String {
        let cleaned = s.to_lowercase()
            // Strip common symbols
            .replace('.', "")
            .replace('\'', "")
            .replace(':', "")
            .replace('®', "")
            .replace('™', "")
            // Standardize separators
            .replace('-', " ")
            .replace('_', " ");

        // Word-by-word substitution to prevent partial string matching issues (like "it" -> "1t")
        let words: Vec<String> = cleaned
            .split_whitespace()
            .map(|w| {
                match w {
                    "ix" => "9".to_string(),
                    "viii" => "8".to_string(),
                    "vii" => "7".to_string(),
                    "vi" => "6".to_string(),
                    "v" => "5".to_string(),
                    "iv" => "4".to_string(),
                    "iii" => "3".to_string(),
                    "ii" => "2".to_string(),
                    "i" => "1".to_string(),
                    "civ" => "civilization".to_string(),
                    _ => w.to_string(),
                }
            })
            .collect();

        words.join(" ")
    }

    /// Reusable professional scoring algorithm supporting exactness-boost and Levenshtein fuzzy search
    pub fn calculate_relevance_score(&self, query: &str, name: &str) -> usize {
        let q_norm = self.normalize_string(query);
        let n_norm = self.normalize_string(name);

        if q_norm == n_norm {
            // Tier 1: Normalized Exact Match (Highest Priority!)
            0
        } else if n_norm.starts_with(&q_norm) {
            // Tier 2: Normalized Prefix Match (Shorter names come first)
            // Example: "The Witch" -> "The Witch" (score 1) sorts BEFORE "The Witcher" (score 4)
            1 + (n_norm.len() - q_norm.len())
        } else if n_norm.contains(&q_norm) {
            // Tier 3: Normalized Substring Match (Early position boosts priority)
            let pos = n_norm.find(&q_norm).unwrap_or(0);
            100 + pos + (n_norm.len() - q_norm.len())
        } else {
            // Tier 4: Fuzzy Levenshtein Distance on normalized strings
            let dist = self.levenshtein_distance(&q_norm, &n_norm);
            
            // Allow larger Levenshtein matching on longer queries
            let max_dist = if q_norm.len() > 8 { 3 } else { 2 };

            if dist <= max_dist && q_norm.len() > 3 {
                1000 + dist
            } else {
                10000 // Not a reasonable match, deprioritize
            }
        }
    }

    /// Helper utility to calculate Levenshtein distance (edit distance) between two strings
    fn levenshtein_distance(&self, s1: &str, s2: &str) -> usize {
        let len1 = s1.chars().count();
        let len2 = s2.chars().count();
        
        let mut dp = vec![vec![0; len2 + 1]; len1 + 1];

        for i in 0..=len1 {
            dp[i][0] = i;
        }
        for j in 0..=len2 {
            dp[0][j] = j;
        }

        for (i, c1) in s1.chars().enumerate() {
            for (j, c2) in s2.chars().enumerate() {
                if c1 == c2 {
                    dp[i + 1][j + 1] = dp[i][j];
                } else {
                    dp[i + 1][j + 1] = 1 + std::cmp::min(
                        dp[i][j + 1], // Deletion
                        std::cmp::min(
                            dp[i + 1][j], // Insertion
                            dp[i][j] // Substitution
                        )
                    );
                }
            }
        }

        dp[len1][len2]
    }

    /// Collect the merged Hubcap availability set for a query.
    ///
    /// For each query variant (the original plus its first alias expansion,
    /// e.g. "gta" → "grand theft auto") both Hubcap endpoints are queried in
    /// parallel via `HubcapClient::search_all`, and all hits are merged by
    /// app id. The whole lookup is bounded by `HUBCAP_SEARCH_BUDGET_MS`;
    /// any failure mode (no key, timeout, endpoint errors) degrades to an
    /// empty set so the Steam catalog still renders on its own.
    async fn collect_hubcap_hits(
        &self,
        query: &str,
        hubcap_client: Option<&HubcapClient>,
    ) -> Vec<crate::providers::hubcap::HubcapGameItem> {
        let Some(client) = hubcap_client else {
            return Vec::new();
        };

        let variants = aliases::primary_variants(query);
        let lookup = async {
            // At most 2 variants by design (original + first alias expansion);
            // both run concurrently — the match fixes the arity for tokio::join!
            // without pulling in a futures-crate dependency.
            let batches: Vec<Vec<crate::providers::hubcap::HubcapGameItem>> = match variants.len() {
                2 => {
                    let (first, second) = tokio::join!(
                        client.search_all(&variants[0]),
                        client.search_all(&variants[1]),
                    );
                    vec![first, second]
                }
                n if n == 1 => vec![client.search_all(&variants[0]).await],
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
                    "[Hubcap] Search for '{}' ({} variant(s)) returned {} games",
                    query, variants.len(), games.len()
                );
                games
            }
            Err(_) => {
                eprintln!("[Hubcap] Search timeout for '{}' ({}ms)", query, HUBCAP_SEARCH_BUDGET_MS);
                Vec::new()
            }
        }
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
    pub async fn search_store(&self, query: &str, hubcap_client: Option<HubcapClient>, show_store_dlcs: bool, show_store_nsfw: bool, show_store_delisted: bool) -> Result<Vec<UnifiedStoreGame>, String> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 1. Fetch from Steam and Hubcap in parallel
        let steam_future = self.steam_store.search_catalog(query);

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

        // 2. Create a fast O(1) lookup set of available app IDs on Hubcap
        let available_ids: HashSet<u32> = hubcap_res.iter().map(|g| g.app_id).collect();

        let mut unified_list = Vec::new();
        let mut added_ids = HashSet::new();

        // 3. Populate unified list with Steam search results and overlay manifest availability.
        // DRM/Denuvo enrichment is intentionally NOT done here: it is slower Steam appdetails
        // metadata and is fetched by the frontend after first results are already rendered.
        for item in steam_items {
            let has_manifest = available_ids.contains(&item.id);
            unified_list.push(UnifiedStoreGame {
                id: item.id,
                name: item.name,
                app_id: item.id.to_string(),
                has_manifest,
                has_denuvo: false,
                has_nsfw: false,
                has_delisted: false,
                image_url: item.image_url,
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
                });
                added_ids.insert(hg.app_id);
            }
        }
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
            unified_list.retain(|game| best_score(&game.name) < 10000);
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
            store_items::fetch_store_items(all_ids).await
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
