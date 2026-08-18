use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;

use crate::manifest::pins::DepotManifestPin;
use crate::providers::http;
use crate::versioning::error::VersionError;
use crate::versioning::sources::{BoxFuture, BuildDetailsSource};

const BASE_URL: &str = "https://depotbox.org/api/depotboxtool/v1";
/// Total request timeout. A healthy Depotbox lookup answers in a few seconds;
/// 45 s keeps the worst case (slow network, large build) bounded so the UI
/// never appears to load forever (SFF uses connect 10 s / read 120 s).
const DEPOTBOX_TIMEOUT_SECONDS: u64 = 45;

/// Depotbox `build-details` endpoint: BuildID → the (depot, manifest) pins of
/// that build. Requires an `x-api-key` header (see `sources::mod` for token
/// resolution). Responses are parsed defensively: provider payload shapes
/// shift over time and a single bad row must never fail the whole lookup.
pub struct DepotboxSource {
    client: reqwest::Client,
}

impl DepotboxSource {
    pub fn new(token: String) -> Self {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(token.trim()) {
            headers.insert("x-api-key", value);
        }
        Self {
            client: http::build_client_with_headers(DEPOTBOX_TIMEOUT_SECONDS, headers),
        }
    }
}

impl BuildDetailsSource for DepotboxSource {
    fn pins_for_build(&self, build_id: u64) -> BoxFuture<'_, Result<Vec<DepotManifestPin>, VersionError>> {
        Box::pin(async move {
            let url = format!("{}/build-details?build_id={}", BASE_URL, build_id);
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| VersionError::source("Depotbox", e))?;
            if !resp.status().is_success() {
                return Err(VersionError::SourceUnavailable {
                    source: "Depotbox",
                    detail: format!("HTTP {}", resp.status().as_u16()),
                });
            }
            let json: Value = resp
                .json()
                .await
                .map_err(|e| VersionError::parse("Depotbox response", e))?;
            parse_build_details(&json)
        })
    }
}

/// Maps Depotbox error payloads to domain errors and extracts valid pins.
pub fn parse_build_details(json: &Value) -> Result<Vec<DepotManifestPin>, VersionError> {
    if json.get("success").and_then(Value::as_bool) != Some(true) {
        let error_code = json
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error")
            .to_string();
        let message = json
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        return Err(match error_code.as_str() {
            "invalid_build_id" | "not_found" => VersionError::BuildNotFound,
            "unauthorized" | "invalid_token" => VersionError::TokenInvalid,
            "vpn_blocked" | "proxy_blocked" => VersionError::SourceUnavailable {
                source: "Depotbox",
                detail: if message.is_empty() {
                    "the service blocked this network (VPN/proxy detected)".to_string()
                } else {
                    message
                },
            },
            _ => VersionError::SourceUnavailable {
                source: "Depotbox",
                detail: if message.is_empty() {
                    error_code
                } else {
                    format!("{}: {}", error_code, message)
                },
            },
        });
    }

    let depots = json
        .get("depots")
        .and_then(Value::as_array)
        .ok_or_else(|| VersionError::parse("Depotbox response", "missing `depots` array"))?;

    let mut pins: Vec<DepotManifestPin> = Vec::new();
    for entry in depots {
        let depot = value_to_string(entry.get("depot_id")).unwrap_or_default();
        let manifest = value_to_string(entry.get("manifest_id")).unwrap_or_default();
        let all_digits =
            |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        if !all_digits(&depot) || !all_digits(&manifest) {
            continue;
        }
        // Same sanity bounds SFF applies: depots are ≤ 12 digits, manifest
        // GIDs ≤ 22. Anything outside is a malformed row, not a real pin.
        if depot.len() > 12 || manifest.len() > 22 {
            continue;
        }
        let Ok(depot_id) = depot.parse::<u32>() else {
            continue;
        };
        pins.push(DepotManifestPin {
            depot_id,
            manifest_id: manifest,
        });
    }

    pins.sort_by_key(|pin| pin.depot_id);
    pins.dedup_by_key(|pin| pin.depot_id);
    if pins.is_empty() {
        return Err(VersionError::BuildNotFound);
    }
    Ok(pins)
}

/// Depotbox returns ids as strings or numbers depending on payload version.
fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_success_shape_with_numeric_ids() {
        let json = json!({
            "success": true,
            "depots": [
                { "depot_id": 2347770, "manifest_id": "2991528520052157173" },
                { "depot_id": 2347771, "manifest_id": 8124921270987929782u64 },
            ]
        });
        let pins = parse_build_details(&json).unwrap();
        assert_eq!(pins.len(), 2);
        assert_eq!(pins[0].depot_id, 2347770);
        assert_eq!(pins[0].manifest_id, "2991528520052157173");
    }

    #[test]
    fn drops_malformed_rows_keeps_valid() {
        let json = json!({
            "success": true,
            "depots": [
                { "depot_id": "not-a-number", "manifest_id": "123" },
                { "depot_id": 2347770, "manifest_id": "" },
                { "depot_id": 2347771, "manifest_id": "abc" },
                { "depot_id": "123456789012345", "manifest_id": "1" },
                { "depot_id": 2347772, "manifest_id": "2991528520052157173" }
            ]
        });
        let pins = parse_build_details(&json).unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].depot_id, 2347772);
    }

    #[test]
    fn dedupes_by_depot() {
        let json = json!({
            "success": true,
            "depots": [
                { "depot_id": 10, "manifest_id": "111" },
                { "depot_id": 10, "manifest_id": "111" }
            ]
        });
        let pins = parse_build_details(&json).unwrap();
        assert_eq!(pins.len(), 1);
    }

    #[test]
    fn maps_error_payloads() {
        assert!(matches!(
            parse_build_details(&json!({ "success": false, "error": "invalid_build_id" })),
            Err(VersionError::BuildNotFound)
        ));
        assert!(matches!(
            parse_build_details(&json!({ "success": false, "error": "unauthorized" })),
            Err(VersionError::TokenInvalid)
        ));
        assert!(matches!(
            parse_build_details(&json!({ "success": false, "error": "vpn_blocked", "message": "Turn off your VPN" })),
            Err(VersionError::SourceUnavailable { source: "Depotbox", .. })
        ));
        assert!(matches!(
            parse_build_details(&json!({ "success": true, "depots": [] })),
            Err(VersionError::BuildNotFound)
        ));
    }
}
