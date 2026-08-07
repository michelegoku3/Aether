use crate::store::aliases::{expanded_queries, primary_variants, MAX_VARIANTS};

#[test]
fn empty_or_blank_query_yields_nothing() {
    assert!(expanded_queries("").is_empty());
    assert!(expanded_queries("   ").is_empty());
}

#[test]
fn plain_query_has_only_the_original_variant() {
    assert_eq!(expanded_queries("cyberpunk 2077"), vec!["cyberpunk 2077"]);
}

#[test]
fn whole_query_alias_expands() {
    let variants = expanded_queries("gta");
    assert_eq!(variants[0], "gta");
    assert!(variants.contains(&"grand theft auto".to_string()));
}

#[test]
fn token_alias_expands_in_place() {
    let variants = expanded_queries("gta san andreas");
    assert!(variants.contains(&"grand theft auto san andreas".to_string()));
}

#[test]
fn expansion_is_case_insensitive_and_deduped() {
    let variants = expanded_queries("GTA");
    assert_eq!(variants[0], "GTA");
    let lowercase: Vec<String> = variants.iter().map(|v| v.to_lowercase()).collect();
    let mut deduped = lowercase.clone();
    deduped.dedup();
    deduped.sort();
    let mut sorted = lowercase.clone();
    sorted.sort();
    assert_eq!(sorted, deduped);
}

#[test]
fn fan_out_is_capped() {
    let variants = expanded_queries("cs go gta re cod");
    assert!(variants.len() <= MAX_VARIANTS);
}

#[test]
fn primary_variants_returns_at_most_two() {
    assert!(primary_variants("csgo").len() <= 2);
    assert_eq!(primary_variants("stray"), vec!["stray"]);
}

#[test]
fn alias_with_punctuation_expands() {
    // Gta con punteggiatura deve comunque espandere via clean_token
    let variants = expanded_queries("GTA: San Andreas");
    assert!(variants.iter().any(|v| v.to_lowercase().contains("grand theft auto")));
}

#[test]
fn civ_alias_expands_to_civilization() {
    let variants = expanded_queries("civ 6");
    assert!(variants.contains(&"civilization 6".to_string()));
    // Whole query
    let variants = expanded_queries("civ");
    assert!(variants.contains(&"civilization".to_string()));
}

#[test]
fn new_aliases_are_present() {
    // Verifica che molti nuovi alias introdotti siano effettivamente espansi
    assert!(expanded_queries("r6 siege").contains(&"rainbow six siege siege".to_string()) || expanded_queries("r6").contains(&"rainbow six siege".to_string()));
    assert!(expanded_queries("ow2").contains(&"overwatch 2".to_string()));
    assert!(expanded_queries("rdr2").contains(&"red dead redemption 2".to_string()));
    assert!(expanded_queries("tlou").contains(&"the last of us".to_string()));
    assert!(expanded_queries("botw").contains(&"breath of the wild".to_string()));
    assert!(expanded_queries("acnh").contains(&"animal crossing new horizons".to_string()));
}

#[test]
fn take_me_to_the_dungeon_no_alias() {
    let variants = expanded_queries("Take Me To The Dungeon!!");
    assert_eq!(variants, vec!["Take Me To The Dungeon!!"]);
}
