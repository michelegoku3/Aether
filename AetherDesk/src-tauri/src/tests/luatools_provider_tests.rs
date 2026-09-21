use std::collections::HashMap;

use crate::providers::luatools::{choose_available_source, describe_api_error, rank_available_sources};
use reqwest::StatusCode;

#[test]
fn chooses_available_source_by_stable_preference() {
    let statuses = HashMap::from([
        ("Sushi".to_string(), "available".to_string()),
        ("Ryuu".to_string(), "available".to_string()),
        ("Luie".to_string(), "unavailable".to_string()),
    ]);

    assert_eq!(choose_available_source(&statuses).as_deref(), Some("Ryuu"));
}

#[test]
fn source_status_matching_is_case_insensitive() {
    let statuses = HashMap::from([("rYuU".to_string(), "AVAILABLE".to_string())]);

    assert_eq!(choose_available_source(&statuses).as_deref(), Some("rYuU"));
}

#[test]
fn falls_back_deterministically_for_future_source_names() {
    let statuses = HashMap::from([
        ("Zulu".to_string(), "available".to_string()),
        ("Alpha".to_string(), "available".to_string()),
    ]);

    assert_eq!(choose_available_source(&statuses).as_deref(), Some("Alpha"));
}

#[test]
fn returns_none_when_no_source_is_available() {
    let statuses = HashMap::from([
        ("Luie".to_string(), "unavailable".to_string()),
        ("Ryuu".to_string(), "offline".to_string()),
    ]);

    assert_eq!(choose_available_source(&statuses), None);
}

#[test]
fn api_errors_are_described_like_the_official_client() {
    assert!(describe_api_error(StatusCode::UNAUTHORIZED, "").contains("login required"));
    assert!(describe_api_error(StatusCode::TOO_MANY_REQUESTS, "").contains("25/day"));
    assert_eq!(
        describe_api_error(StatusCode::TOO_MANY_REQUESTS, r#"{"error":"Rate limit exceeded"}"#),
        "Rate limit exceeded"
    );
    assert_eq!(
        describe_api_error(StatusCode::NOT_FOUND, r#"{"error":"Manifest not found"}"#),
        "Manifest not found"
    );
    assert_eq!(
        describe_api_error(StatusCode::BAD_GATEWAY, "<html>\nupstream down</html>"),
        "<html> upstream down</html>"
    );
}

#[test]
fn ranking_prefers_sources_that_ship_manifests_and_keeps_luie_as_fallback() {
    let statuses = HashMap::from([
        ("Luie".to_string(), "available".to_string()),
        ("Ryuu".to_string(), "available".to_string()),
        ("Zulu".to_string(), "available".to_string()),
        ("Alpha".to_string(), "available".to_string()),
        ("Sushi".to_string(), "offline".to_string()),
    ]);

    // Zip-shipping sources first, Luie (bare Lua) after them, unknown names
    // last in alphabetical order; unavailable ones never appear.
    assert_eq!(
        rank_available_sources(&statuses),
        vec!["Ryuu".to_string(), "Luie".to_string(), "Alpha".to_string(), "Zulu".to_string()]
    );
    assert_eq!(choose_available_source(&statuses).as_deref(), Some("Ryuu"));

    let luie_only = HashMap::from([
        ("Luie".to_string(), "available".to_string()),
        ("Ryuu".to_string(), "unavailable".to_string()),
    ]);
    assert_eq!(rank_available_sources(&luie_only), vec!["Luie".to_string()]);
}
