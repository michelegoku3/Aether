use crate::commands::home_links::{build_csrinru_url, build_gcw_url, build_onlinefix_url};

// Helper to extract query values for assertions without depending on private fns.
fn extract_onlinefix_query(url: &str) -> String {
    // URL is https://online-fix.me/...?do=search&subaction=search&story=<query>
    url.split("story=").nth(1).unwrap_or("").to_string()
}
fn extract_gcw_query(url: &str) -> String {
    url.split("q=").nth(1).unwrap_or("").split('&').next().unwrap_or("").to_string()
}
fn extract_csrinru_keywords(url: &str) -> String {
    url.split("keywords=").nth(1).unwrap_or("").split('&').next().unwrap_or("").to_string()
}

#[test]
fn onlinefix_strips_tm_and_keeps_095_form_encoding() {
    let url = build_onlinefix_url("DARK SOULS™ III");
    let q = extract_onlinefix_query(&url);

    assert_eq!(q, "dark+souls+iii");
    assert!(!q.contains("%E2%84%A2"), "TM should be stripped, got {}", q);
}

#[test]
fn onlinefix_keeps_real_subtitle_after_colon() {
    let url = build_onlinefix_url("Call of Duty: Black Ops II");
    let q = extract_onlinefix_query(&url);

    assert_eq!(q, "call+of+duty+black+ops+ii");
    assert_ne!(q, "call+of+duty");
}

#[test]
fn onlinefix_strips_editions_without_dropping_real_subtitle() {
    let sekiro = build_onlinefix_url("Sekiro™: Shadows Die Twice - GOTY Edition");
    let sekiro_q = extract_onlinefix_query(&sekiro);

    assert_eq!(sekiro_q, "sekiro+shadows+die+twice");
    assert!(!sekiro_q.to_lowercase().contains("goty"));
    assert!(!sekiro_q.contains("%E2%84%A2"));
}

#[test]
fn onlinefix_rv_there_yet_uses_the_095_cleanup() {
    let url = build_onlinefix_url("RV There Yet?");
    let q = extract_onlinefix_query(&url);

    // 0.9.5-style cleanup strips punctuation from the search text and uses
    // form-urlencoded spaces. This test documents the restored behavior.
    assert_eq!(q, "rv+there+yet");
}

#[test]
fn gcw_uses_generic_search_not_brittle_direct_slugs() {
    let url = build_gcw_url("Call of Duty: Black Ops II");
    let q = extract_gcw_query(&url);

    assert!(url.starts_with("https://gamecopyworld.eu/games/search_results.shtml?q="));
    assert_eq!(q, "call+of+duty+black+ops+ii");
    assert!(!url.contains("pc_call_of_duty_black_ops_2.shtml"));
    assert!(!url.contains("pc_call_of_duty_9.shtml"));
}

#[test]
fn gcw_search_query_cleans_titles_without_specific_exceptions() {
    let goat = build_gcw_url("Goat Simulator: Remastered");
    assert_eq!(extract_gcw_query(&goat), "goat+simulator+remastered");

    let dark_souls = build_gcw_url("DARK SOULS™ III");
    assert_eq!(extract_gcw_query(&dark_souls), "dark+souls+iii");
}

#[test]
fn csrinru_davigo_strips_only_platform_subtitle() {
    let url = build_csrinru_url("DAVIGO: VR vs. PC");
    let kw = extract_csrinru_keywords(&url);

    assert_eq!(kw, "davigo");
}

#[test]
fn csrinru_sekiro_strips_tm_and_goty_but_keeps_real_subtitle() {
    let url = build_csrinru_url("Sekiro™: Shadows Die Twice - GOTY Edition");
    let kw = extract_csrinru_keywords(&url);

    assert_eq!(kw, "sekiro+shadows+die+twice");
    assert!(!kw.to_lowercase().contains("goty"));
    assert!(!kw.contains("%E2%84%A2"));
}

#[test]
fn csrinru_stray_titleonly_topics_in_game_forum() {
    let url = build_csrinru_url("Stray");
    assert!(url.contains("fid%5B%5D=10"), "should restrict to the main game forum");
    assert!(url.contains("sf=titleonly"), "should be titleonly");
    assert!(url.contains("sr=topics"), "should be topics");
    let kw = extract_csrinru_keywords(&url);
    assert_eq!(kw, "stray");
}

#[test]
fn csrinru_keeps_095_numeric_drop_policy() {
    let url = build_csrinru_url("Crusader Kings 3");
    let kw = extract_csrinru_keywords(&url);

    assert_eq!(kw, "crusader+kings");
}
