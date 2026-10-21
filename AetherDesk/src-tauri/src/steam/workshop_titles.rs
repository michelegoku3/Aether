use crate::providers::http;
use serde::Deserialize;
use std::collections::HashMap;

const TITLE_LOOKUP_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const TITLE_LOOKUP_TIMEOUT_SECONDS: u64 = 5;
const MAX_IDS_PER_REQUEST: usize = 100;

#[derive(Debug, Deserialize)]
struct TitleEnvelope {
    response: Option<TitleResponse>,
}

#[derive(Debug, Deserialize)]
struct TitleResponse {
    #[serde(default)]
    publishedfiledetails: Vec<PublishedFileDetail>,
}

#[derive(Debug, Deserialize)]
struct PublishedFileDetail {
    publishedfileid: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    result: Option<i32>,
}

/// Resolves Workshop item titles through Steam's public published-file API.
///
/// Titles are not stored in `appworkshop_*.acf`; the report needs one batch
/// lookup to show a useful name instead of only the numeric ID. The endpoint is
/// public and does not consume Hubcap quota. A failed lookup is non-fatal: the
/// report falls back to `Mod #<id>`.
pub async fn fetch_titles(workshop_ids: Vec<u64>) -> Result<HashMap<u64, String>, String> {
    let mut unique_ids = workshop_ids;
    unique_ids.sort_unstable();
    unique_ids.dedup();
    if unique_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let client = http::shared_client(TITLE_LOOKUP_TIMEOUT_SECONDS);
    let mut titles = HashMap::new();
    for chunk in unique_ids.chunks(MAX_IDS_PER_REQUEST) {
        let mut form = Vec::with_capacity(chunk.len() + 1);
        form.push(("itemcount".to_string(), chunk.len().to_string()));
        for (index, workshop_id) in chunk.iter().enumerate() {
            form.push((
                format!("publishedfileids[{index}]"),
                workshop_id.to_string(),
            ));
        }

        let response = client
            .post(TITLE_LOOKUP_URL)
            .form(&form)
            .send()
            .await
            .map_err(|error| format!("Steam Workshop title lookup failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "Steam Workshop title lookup returned HTTP {}",
                response.status()
            ));
        }
        let envelope = response.json::<TitleEnvelope>().await.map_err(|error| {
            format!("Steam Workshop title lookup returned invalid JSON: {error}")
        })?;
        titles.extend(collect_titles(envelope));
    }

    Ok(titles)
}

fn collect_titles(envelope: TitleEnvelope) -> HashMap<u64, String> {
    let Some(response) = envelope.response else {
        return HashMap::new();
    };
    response
        .publishedfiledetails
        .into_iter()
        .filter_map(|detail| {
            if detail.result.unwrap_or(1) != 1 {
                return None;
            }
            let workshop_id = detail.publishedfileid.parse::<u64>().ok()?;
            let title = detail.title?.trim().to_string();
            (!title.is_empty()).then_some((workshop_id, title))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collects_titles_and_skips_failed_or_empty_entries() {
        let envelope: TitleEnvelope = serde_json::from_value(json!({
            "response": {
                "publishedfiledetails": [
                    {
                        "publishedfileid": "3809288964",
                        "title": "Uniforms of the Realm",
                        "result": 1
                    },
                    {
                        "publishedfileid": "111",
                        "title": "Hidden because result is zero",
                        "result": 0
                    },
                    {
                        "publishedfileid": "222",
                        "title": "   ",
                        "result": 1
                    }
                ]
            }
        }))
        .unwrap();

        let titles = collect_titles(envelope);

        assert_eq!(
            titles.get(&3809288964).map(String::as_str),
            Some("Uniforms of the Realm")
        );
        assert_eq!(titles.len(), 1);
    }

    #[test]
    fn missing_response_is_an_empty_lookup() {
        let envelope: TitleEnvelope = serde_json::from_value(json!({})).unwrap();

        assert!(collect_titles(envelope).is_empty());
    }
}
