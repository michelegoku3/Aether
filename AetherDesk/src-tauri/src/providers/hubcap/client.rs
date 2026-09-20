use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use crate::providers::http;
use super::types::HUBCAP_TIMEOUT_SECONDS;

#[derive(Clone)]
pub struct HubcapClient {
    pub(crate) api_key: String,
    pub(crate) client: reqwest::Client,
}

impl HubcapClient {
    pub fn new(api_key: String) -> Self {
        let api_key = api_key.trim().to_string();
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", api_key)) {
            headers.insert(AUTHORIZATION, value);
        }
        Self {
            api_key,
            client: http::build_client_with_headers(HUBCAP_TIMEOUT_SECONDS, headers),
        }
    }

    pub(crate) fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
            headers.insert(AUTHORIZATION, value);
        }
        headers
    }

    /// Server-side statuses that mean "this query is not answerable" rather than
    /// "the app is broken".
    pub(crate) fn is_soft_failure(status: reqwest::StatusCode) -> bool {
        matches!(status.as_u16(), 400 | 500 | 503)
    }
}
