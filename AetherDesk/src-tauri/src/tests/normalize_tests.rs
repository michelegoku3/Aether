use crate::store::normalize::{normalize_string, sanitize_query_for_hubcap};

#[test]
fn normalize_strips_punctuation_to_space() {
    assert_eq!(normalize_string("Take Me To The Dungeon!!"), "take me to the dungeon");
    assert_eq!(normalize_string("GTA: San Andreas"), "gta san andreas");
    assert_eq!(normalize_string("LEGO® Batman™: Legacy"), "lego batman legacy");
    assert_eq!(normalize_string("Cyberpunk 2077"), "cyberpunk 2077");
    assert_eq!(normalize_string("BioShock - Remastered"), "bioshock remastered");
    assert_eq!(normalize_string("It Takes Two"), "it takes two");
    assert_eq!(normalize_string("Dark Souls II"), "dark souls 2");
}

#[test]
fn normalize_roman_conversion() {
    assert_eq!(normalize_string("Dark Souls II"), "dark souls 2");
    assert_eq!(normalize_string("ix"), "9");
    // civ è ora alias, non roman — deve rimanere civ
    assert_eq!(normalize_string("Civ VI"), "civ 6");
}

#[test]
fn normalize_civ_not_converted() {
    // civ -> civilization è alias, non normalizzazione
    assert_eq!(normalize_string("civ"), "civ");
    assert_eq!(normalize_string("civ 6"), "civ 6");
}

#[test]
fn normalize_collapses_whitespace() {
    assert_eq!(normalize_string("  hello   world  "), "hello world");
    assert_eq!(normalize_string("hello---world"), "hello world");
}

#[test]
fn sanitize_hubcap_basic() {
    assert_eq!(
        sanitize_query_for_hubcap("Take Me To The Dungeon!!"),
        Some("Take Me To The Dungeon".to_string())
    );
    assert_eq!(
        sanitize_query_for_hubcap("GTA: San Andreas"),
        Some("GTA San Andreas".to_string())
    );
    assert_eq!(sanitize_query_for_hubcap("Cyberpunk 2077"), None);
    assert_eq!(sanitize_query_for_hubcap("   "), None);
    assert_eq!(sanitize_query_for_hubcap("!!"), None);
}

#[test]
fn sanitize_hubcap_preserves_alphanumeric() {
    assert_eq!(sanitize_query_for_hubcap("Stray"), None);
    assert_eq!(
        sanitize_query_for_hubcap("Arma 3: - Apex !!"),
        Some("Arma 3 Apex".to_string())
    );
}

#[test]
fn normalize_take_me_exact() {
    // Il titolo con !! e senza !! collassano allo stesso normalize → score 0
    assert_eq!(
        normalize_string("Take Me To The Dungeon!!"),
        normalize_string("Take Me To The Dungeon")
    );
}
