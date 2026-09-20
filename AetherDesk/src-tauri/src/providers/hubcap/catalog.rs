use std::collections::HashSet;

use super::client::HubcapClient;
use super::types::{
    HubcapGameItem, HubcapLibraryResponse, HubcapSearchResponse, BASE_URL,
    CATALOG_SEARCH_LIMIT, LIBRARY_SEARCH_LIMIT,
};

impl HubcapClient {
    /// Broad recall search: `GET /library?search=…` with a large page size.
    pub async fn search_library(&self, query: &str) -> Result<Vec<HubcapGameItem>, String> {
        let url = format!("{}/library", BASE_URL);
        let params: Vec<(&str, String)> = vec![
            ("search", query.to_string()),
            ("limit", LIBRARY_SEARCH_LIMIT.to_string()),
        ];
        let response = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(&params)
            .send()
            .await
            .map_err(|e| format!("Hubcap API network error: {}", e))?;

        if Self::is_soft_failure(response.status()) {
            crate::desk_log_warn!(
                "hubcap",
                "Library search soft-failed HTTP {} query_len={}",
                response.status(),
                query.len()
            );
            return Ok(Vec::new());
        }

        if !response.status().is_success() {
            return Ok(Vec::new());
        }

        let data = response
            .json::<HubcapLibraryResponse>()
            .await
            .map_err(|e| format!("Failed to parse Hubcap /library response: {}", e))?;

        Ok(data.games)
    }

    /// Precise search: `GET /search?q=…`.
    pub async fn search_games(&self, query: &str) -> Result<Vec<HubcapGameItem>, String> {
        let url = format!("{}/search", BASE_URL);
        let mut params: Vec<(&str, String)> = vec![
            ("q", query.to_string()),
            ("limit", CATALOG_SEARCH_LIMIT.to_string()),
        ];
        if query.trim().parse::<u32>().is_ok() {
            params.push(("appid", "true".to_string()));
        }

        let response = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(&params)
            .send()
            .await
            .map_err(|e| format!("Hubcap API network error: {}", e))?;

        if Self::is_soft_failure(response.status()) {
            crate::desk_log_warn!(
                "hubcap",
                "Catalog search soft-failed HTTP {} query_len={}",
                response.status(),
                query.len()
            );
            return Ok(Vec::new());
        }

        if !response.status().is_success() {
            return Ok(Vec::new());
        }

        let data = response
            .json::<HubcapSearchResponse>()
            .await
            .map_err(|e| format!("Failed to parse Hubcap /search response: {}", e))?;

        Ok(data.into_items())
    }

    /// One logical search against Hubcap, backed by two requests with different
    /// endpoints issued in parallel (`/library` + `/search`).
    pub async fn search_all(&self, query: &str) -> Vec<HubcapGameItem> {
        let (library_result, games_result) = tokio::join!(
            self.search_library(query),
            self.search_games(query),
        );

        let mut merged: Vec<HubcapGameItem> = Vec::new();
        let mut seen_ids: HashSet<u32> = HashSet::new();

        for result in [library_result, games_result] {
            match result {
                Ok(items) => {
                    for item in items {
                        if item.app_id != 0 && seen_ids.insert(item.app_id) {
                            merged.push(item);
                        }
                    }
                }
                Err(error) => {
                    crate::desk_log_warn!(
                        "hubcap",
                        "Search endpoint failed query_len={}: {}",
                        query.len(),
                        error
                    );
                }
            }
        }

        merged
    }
}
