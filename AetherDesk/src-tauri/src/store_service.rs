use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use crate::steam_store::SteamStore;
use crate::hubcap_client::HubcapClient;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UnifiedStoreGame {
    pub id: u32, // required to act as the unique React key
    pub name: String,
    pub appId: String,
    pub has_manifest: bool,
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

    /// Queries Steam and Hubcap in parallel, then merges results with high performance
    pub async fn search_store(&self, query: &str, hubcap_client: Option<HubcapClient>) -> Result<Vec<UnifiedStoreGame>, String> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 1. Fetch from Steam and Hubcap in parallel (Double speed via Tokio async!)
        let steam_future = self.steam_store.search_catalog(query);
        
        let hubcap_future = async {
            match &hubcap_client {
                Some(client) => client.search_library(query).await.unwrap_or_default(),
                None => Vec::new(),
            }
        };

        let (steam_res, hubcap_res) = tokio::join!(steam_future, hubcap_future);
        let steam_items = steam_res?;

        // 2. Create a fast O(1) lookup set of available app IDs on Hubcap
        let available_ids: HashSet<u32> = hubcap_res.iter().map(|g| g.app_id).collect();

        let mut unified_list = Vec::new();
        let mut added_ids = HashSet::new();

        // 3. Populate unified list with Steam search results and overlay manifest availability
        for item in steam_items {
            let has_manifest = available_ids.contains(&item.id);
            unified_list.push(UnifiedStoreGame {
                id: item.id,
                name: item.name,
                appId: item.id.to_string(),
                has_manifest,
            });
            added_ids.insert(item.id);
        }

        // 4. Fallback: If Hubcap returned matches that are NOT in Steam results, append them
        // (This covers delisted legacy classics)
        for hg in hubcap_res {
            if !added_ids.contains(&hg.app_id) {
                unified_list.push(UnifiedStoreGame {
                    id: hg.app_id,
                    name: hg.name,
                    appId: hg.app_id.to_string(),
                    has_manifest: true,
                });
                added_ids.insert(hg.app_id);
            }
        }

        Ok(unified_list)
    }
}
