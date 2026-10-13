use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const DEFAULT_USER_AGENT: &str = "AetherDesk/1.0";

/// Shared connection pools keyed by `(timeout, headers)`.
///
/// `reqwest::Client` is cheap to clone (the pool is shared), but expensive to
/// build (new pool, new TLS state). Every provider goes through here so a
/// typed store search or a depot fan-out reuses pools instead of rebuilding
/// them per keystroke or per row.
static REGISTRY: OnceLock<Mutex<HashMap<ClientKey, reqwest::Client>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClientKey {
    timeout_secs: u64,
    headers: Vec<(String, Vec<u8>)>,
}

fn fingerprint(headers: &HeaderMap) -> Vec<(String, Vec<u8>)> {
    let mut items: Vec<(String, Vec<u8>)> = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_string(), value.as_bytes().to_vec()))
        .collect();
    items.sort();
    items
}

/// Shared client for a timeout: same pool on every call with the same timeout.
pub fn shared_client(timeout_secs: u64) -> reqwest::Client {
    shared_client_with_headers(timeout_secs, HeaderMap::new())
}

/// Shared client for a timeout plus custom headers (e.g. Authorization).
/// The default User-Agent is applied exactly like `build_client_with_headers`.
pub fn shared_client_with_headers(timeout_secs: u64, extra_headers: HeaderMap) -> reqwest::Client {
    let mut headers = extra_headers;
    headers
        .entry(USER_AGENT)
        .or_insert(HeaderValue::from_static(DEFAULT_USER_AGENT));
    let key = ClientKey {
        timeout_secs,
        headers: fingerprint(&headers),
    };
    if let Ok(guard) = REGISTRY.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        if let Some(client) = guard.get(&key) {
            return client.clone();
        }
    }
    let client = build_client_with_headers(timeout_secs, headers);
    // A poisoned lock only costs one unshared build; never panic here.
    if let Ok(mut guard) = REGISTRY.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        guard.insert(key, client.clone());
    }
    client
}

/// Low-level constructor shared by the registry below: timeout plus
/// default User-Agent, with a fallback to a plain client.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_ignores_header_insertion_order() {
        let mut first = HeaderMap::new();
        first.insert("x-api-key", HeaderValue::from_static("secret"));
        first.insert("x-other", HeaderValue::from_static("1"));
        let mut second = HeaderMap::new();
        second.insert("x-other", HeaderValue::from_static("1"));
        second.insert("x-api-key", HeaderValue::from_static("secret"));
        assert_eq!(fingerprint(&first), fingerprint(&second));
    }

    #[test]
    fn fingerprint_separates_timeouts_values_and_names() {
        let base = HeaderMap::new();
        let key = |timeout: u64, headers: &HeaderMap| ClientKey {
            timeout_secs: timeout,
            headers: fingerprint(headers),
        };
        assert_ne!(key(5, &base), key(10, &base));
        let mut other = HeaderMap::new();
        other.insert("x-api-key", HeaderValue::from_static("different"));
        let mut named = HeaderMap::new();
        named.insert("x-api-key", HeaderValue::from_static("secret"));
        assert_ne!(fingerprint(&other), fingerprint(&named));
        assert_ne!(key(5, &other), key(5, &named));
    }

    #[test]
    fn shared_clients_build_and_reuse_without_panic() {
        let first = shared_client(7);
        let second = shared_client(7);
        let _ = (first, second);
        let keyed = shared_client_with_headers(7, HeaderMap::new());
        let _ = keyed;
    }
}
