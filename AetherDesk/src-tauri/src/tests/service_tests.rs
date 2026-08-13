use crate::store::service::StoreService;
use crate::store::normalize::{normalize_string, sanitize_query_for_hubcap};

#[test]
fn take_me_scoring_is_exact_after_normalize() {
    let svc = StoreService::new();
    assert_eq!(svc.calculate_relevance_score("Take Me To The Dungeon!!", "Take Me To The Dungeon!!"), 0);
    assert_eq!(svc.calculate_relevance_score("Take Me To The Dungeon", "Take Me To The Dungeon!!"), 0);
    assert_eq!(svc.calculate_relevance_score("take me to the dungeon!!", "TAKE ME TO THE DUNGEON"), 0);
}

#[test]
fn scoring_is_exact_prefix_or_substring_only() {
    let svc = StoreService::new();
    assert_eq!(svc.calculate_relevance_score("stray", "Stray"), 0);
    let prefix = svc.calculate_relevance_score("stray", "Stray Cat");
    let substring = svc.calculate_relevance_score("stray", "My Stray Game");
    assert!(prefix > 0 && prefix < 100);
    assert!(substring >= 100 && substring < 1000);
    // Typos are Steam/suggest's job, not local Levenshtein.
    assert_eq!(svc.calculate_relevance_score("stray", "Stary"), 10000);
    assert_eq!(svc.calculate_relevance_score("witchr 3", "The Witcher 3: Wild Hunt"), 10000);
}

#[test]
fn sanitize_generates_extra_query() {
    let q = "Take Me To The Dungeon!!";
    let san = sanitize_query_for_hubcap(q);
    assert_eq!(san, Some("Take Me To The Dungeon".to_string()));
    assert_eq!(normalize_string(q), normalize_string(&san.unwrap()));
}

#[test]
fn civ_alias_not_normalize() {
    assert_eq!(normalize_string("civ 6"), "civ 6");
    assert_ne!(normalize_string("civ 6"), "civilization 6");
    let variants = crate::store::aliases::expanded_queries("civ 6");
    assert!(variants.contains(&"civilization 6".to_string()));
}
