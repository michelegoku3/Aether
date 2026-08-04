use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use std::time::Duration;

const DEFAULT_USER_AGENT: &str = "AetherDesk/1.0";

/// Build a configured `reqwest::Client` with timeout and default User-Agent.
///
/// Every HTTP client in the codebase shares this factory so timeout behaviour,
/// User-Agent, and the fallback-to-default logic stay in one place.
pub fn build_client(timeout_secs: u64) -> reqwest::Client {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));

    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .default_headers(headers)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Build a client with custom headers (e.g. Authorization), still applying
/// the shared timeout and User-Agent.
pub fn build_client_with_headers(timeout_secs: u64, extra_headers: HeaderMap) -> reqwest::Client {
    let mut headers = extra_headers;
    headers
        .entry(USER_AGENT)
        .or_insert(HeaderValue::from_static(DEFAULT_USER_AGENT));

    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .default_headers(headers)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
