use crate::store::service::StoreService;
use crate::store::normalize::{normalize_string, sanitize_query_for_hubcap};

#[test]
fn take_me_scoring_is_exact_after_normalize() {
    let svc = StoreService::new();
    // Con la nuova normalizzazione, !! viene strippato → exact 0
    assert_eq!(svc.calculate_relevance_score("Take Me To The Dungeon!!", "Take Me To The Dungeon!!"), 0);
    assert_eq!(svc.calculate_relevance_score("Take Me To The Dungeon", "Take Me To The Dungeon!!"), 0);
    assert_eq!(svc.calculate_relevance_score("take me to the dungeon!!", "TAKE ME TO THE DUNGEON"), 0);
}

#[test]
fn scoring_tiers_are_respected() {
    let svc = StoreService::new();
    let exact = svc.calculate_relevance_score("stray", "Stray");
    let prefix = svc.calculate_relevance_score("stray", "Stray Cat");
    let substring = svc.calculate_relevance_score("stray", "My Stray Game");
    let fuzzy = svc.calculate_relevance_score("stray", "Stary"); // typo distance 1
    assert_eq!(exact, 0);
    assert!(prefix > 0 && prefix < 100);
    assert!(substring >= 100 && substring < 1000);
    assert!(fuzzy >= 1000 && fuzzy < 10000);
}

#[test]
fn sanitize_generates_extra_query() {
    let q = "Take Me To The Dungeon!!";
    let san = sanitize_query_for_hubcap(q);
    assert_eq!(san, Some("Take Me To The Dungeon".to_string()));
    // Dopo normalize, raw e sanitized collassano allo stesso
    assert_eq!(normalize_string(q), normalize_string(&san.unwrap()));
}

#[test]
fn civ_alias_not_normalize() {
    // civ non è più roman, rimane civ in normalize; l'alias fa il lavoro
    assert_eq!(normalize_string("civ 6"), "civ 6");
    assert_ne!(normalize_string("civ 6"), "civilization 6");
    // Ma expanded_queries deve espandere
    let variants = crate::store::aliases::expanded_queries("civ 6");
    assert!(variants.contains(&"civilization 6".to_string()));
}
