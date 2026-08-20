use std::collections::HashMap;

use crate::providers::luatools::choose_available_source;

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
