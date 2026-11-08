use super::key_status::{HubcapKeyReason, HubcapKeyState, HubcapKeyStatus};
use super::types::HUBCAP_TIMEOUT_SECONDS;
use crate::providers::http;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

#[derive(Clone)]
pub struct HubcapClient {
    pub(crate) api_key: String,
    pub(crate) client: reqwest::Client,
    /// `false` for candidate-key validation from the Settings form: a typo
    /// must not raise the global "saved key expired" warning.
    record_status: bool,
}

impl HubcapClient {
    pub fn new(api_key: &str) -> Self {
        Self::with_recording(api_key, true)
    }

    /// A client whose failures are not published to the global key status.
    /// Used only to validate a candidate key before it is saved.
    pub fn new_quiet(api_key: &str) -> Self {
        Self::with_recording(api_key, false)
    }

    fn with_recording(api_key: &str, record_status: bool) -> Self {
        let api_key = api_key.trim().to_string();
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", api_key)) {
            headers.insert(AUTHORIZATION, value);
        }
        Self {
            api_key,
            client: http::shared_client_with_headers(HUBCAP_TIMEOUT_SECONDS, headers),
            record_status,
        }
    }

    pub(crate) fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
            headers.insert(AUTHORIZATION, value);
        }
        headers
    }

    /// Sends an authenticated request and publishes an authentication failure
    /// before the caller inspects the status, so every Hubcap endpoint feeds
    /// the same key-status warning without per-call plumbing.
    pub(crate) async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let response = request.send().await?;
        self.observe(response.status());
        Ok(response)
    }

    fn observe(&self, status: reqwest::StatusCode) {
        if !self.record_status {
            return;
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            super::key_status::record(
                &self.api_key,
                HubcapKeyState::Inactive,
                Some(HubcapKeyReason::Unauthorized),
            );
        }
    }

    /// Publishes a definitive status obtained from a successful `/user/stats`
    /// answer (`active`) or from `can_make_requests == false` (`inactive`).
    pub(crate) fn record_state(&self, status: &HubcapKeyStatus) {
        if self.record_status {
            super::key_status::record(&self.api_key, status.state, status.reason);
        }
    }

    /// Server-side statuses that mean "this query is not answerable" rather than
    /// "the app is broken".
    pub(crate) fn is_soft_failure(status: reqwest::StatusCode) -> bool {
        matches!(status.as_u16(), 400 | 500 | 503)
    }
}
