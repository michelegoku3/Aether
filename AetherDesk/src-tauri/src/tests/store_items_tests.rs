use crate::steam::store_items::{is_dlc_like, is_nsfw, looks_nsfw_by_name, StoreItemMeta};

fn meta(kind: &str, parent: Option<u32>, delisted_blank: bool, is_nsfw: bool) -> StoreItemMeta {
    StoreItemMeta {
        kind: kind.to_string(),
        parent_appid: parent,
        delisted_blank,
        is_nsfw,
        is_delisted: false,
        release_date_unix: None,
    }
}

#[test]
fn delisted_flag_is_independent_from_blank_signal() {
    let mut m = meta("game", None, false, false);
    assert!(!m.is_delisted);
    m.is_delisted = true;
    assert!(!is_dlc_like(&m));
    assert!(!is_nsfw(&m, "Grand Theft Auto: San Andreas"));
}

#[test]
fn dlc_rules_match_sff() {
    assert!(is_dlc_like(&meta("dlc", Some(107410), false, false)));
    assert!(is_dlc_like(&meta("music", None, false, false)));
    assert!(is_dlc_like(&meta("tool", None, false, false)));
    assert!(is_dlc_like(&meta("", None, true, false)));
    assert!(!is_dlc_like(&meta("rerelease", Some(10), false, false)));
    assert!(!is_dlc_like(&meta("game", None, false, false)));
    assert!(!is_dlc_like(&meta("", None, false, false)));
}

#[test]
fn nsfw_name_heuristic_uses_whole_tokens() {
    assert!(looks_nsfw_by_name("Sex World 3"));
    assert!(looks_nsfw_by_name("Furry Love"));
    assert!(looks_nsfw_by_name("SEXY Party"));
    assert!(!looks_nsfw_by_name("Essex Express"));
    assert!(!looks_nsfw_by_name("Cyberpunk 2077"));
    assert!(!looks_nsfw_by_name("HuniePop"));
}

#[test]
fn nsfw_combines_descriptor_and_name() {
    let flagged = meta("game", None, false, true);
    assert!(is_nsfw(&flagged, "Innocent Title"));
    let clean = meta("game", None, false, false);
    assert!(is_nsfw(&clean, "Furry Love"));
    assert!(!is_nsfw(&clean, "Stardew Valley"));
}

#[test]
fn take_me_is_not_dlc_and_is_nsfw() {
    // 1793250: type game, content_descriptorids [1,3,4,5] => is_nsfw true
    let m = StoreItemMeta {
        kind: "game".to_string(),
        parent_appid: None,
        delisted_blank: false,
        is_nsfw: true,
        is_delisted: false,
        release_date_unix: Some(1688004125),
    };
    assert!(!is_dlc_like(&m));
    assert!(is_nsfw(&m, "Take Me To The Dungeon!!"));
}
