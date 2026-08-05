use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

/// Overall wall-clock budget for the whole Hubcap availability lookup
/// (2 endpoints × up to 2 query variants). The two endpoint calls inside
/// each variant run in parallel, so this is not a per-request timeout:
/// it only guards against the minority of runs where Hubcap hangs.
const HUBCAP_SEARCH_BUDGET_MS: u64 = 6000;
use crate::steam::store::SteamStore;
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
            let mut merged: Vec<crate::providers::hubcap::HubcapGameItem> = Vec::new();
            let mut seen_ids = HashSet::new();
            for variant in &variants {
                for item in client.search_all(variant).await {
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
    pub async fn search_store(&self, query: &str, hubcap_client: Option<HubcapClient>) -> Result<Vec<UnifiedStoreGame>, String> {
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
                image_url: item.image_url,
            });
            added_ids.insert(item.id);
        }

        // 4. Fallback: If Hubcap returned matches that are NOT in Steam results, append them
        for hg in hubcap_res {
            if !added_ids.contains(&hg.app_id) {
                unified_list.push(UnifiedStoreGame {
                    id: hg.app_id,
                    name: hg.name,
                    app_id: hg.app_id.to_string(),
                    has_manifest: true,
                    has_denuvo: false,
                    image_url: String::new(),
                });
                added_ids.insert(hg.app_id);
            }
        }

        // 5. Apply the professional relevance-boosting and fuzzy-sorting!
        let is_numeric = query.trim().parse::<u32>().is_ok();

        if !is_numeric {
            // Score every item against ALL query variants and keep the best one.
            // Without this, hits fetched through an alias expansion
            // ("grand theft auto" found while the user typed "gta") would fail
            // the substring/Levenshtein tiers against the raw query and get
            // filtered out, silently nullifying the alias expansion.
            let score_variants = aliases::expanded_queries(query);
            let mut scored_items: Vec<(usize, UnifiedStoreGame)> = unified_list
                .into_iter()
                .map(|item| {
                    let score = score_variants
                        .iter()
                        .map(|variant| self.calculate_relevance_score(variant, &item.name))
                        .min()
                        .unwrap_or(10000);
                    (score, item)
                })
                .filter(|(score, _)| *score < 10000) // Filter out garbage matches
                .collect();

            scored_items.sort_by_key(|(score, _)| *score);
            
            Ok(scored_items.into_iter().map(|(_, item)| item).collect())
        } else {
            // If it is an App ID, exact or substring matches on digits are kept, no string sorting needed
            Ok(unified_list)
        }
    }
}
