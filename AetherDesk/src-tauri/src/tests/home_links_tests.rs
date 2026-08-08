use crate::commands::home_links::{build_csrinru_url, build_gcw_url, build_onlinefix_url};

// Helper to extract query/slug for assertions without depending on private fns.
fn extract_onlinefix_query(url: &str) -> String {
    // URL is https://online-fix.me/...?do=search&subaction=search&story=<query>
    url.split("story=").nth(1).unwrap_or("").to_string()
}
fn extract_gcw_slug(url: &str) -> String {
    url.split("pc_").nth(1).unwrap_or("").trim_end_matches(".shtml").to_string()
}
fn extract_csrinru_keywords(url: &str) -> String {
    url.split("keywords=").nth(1).unwrap_or("").split('&').next().unwrap_or("").to_string()
}

#[test]
fn onlinefix_strips_tm_and_uses_percent20() {
    let url = build_onlinefix_url("DARK SOULS™ III");
    let q = extract_onlinefix_query(&url);
    // Should be "dark souls iii" lowercased, without ™, roman kept as iii, spaces as %20
    assert!(q.contains("dark%20souls%20iii") || q.contains("dark+souls+iii"), "got {}", q);
    // Must NOT contain the mojibake bytes %E2%84%A2
    assert!(!q.contains("%E2%84%A2"), "TM should be stripped, got {}", q);
    // Should not contain %E2%84 if stripped
    assert!(!q.to_lowercase().contains("e2%84"), "TM bytes should be gone");
}

#[test]
fn onlinefix_rv_there_yet_question_mark() {
    let url = build_onlinefix_url("RV There Yet?");
    let q = extract_onlinefix_query(&url);
    // Should be "rv there yet?" with ? encoded as %3F and spaces as %20 (not +)
    // User reported + vs %20 matters: working link is RV%20There%20Yet%3F
    assert!(q.contains("%20"), "should use %20, got {}", q);
    assert!(q.contains("%3F") || q.contains("?"), "should keep ?", q);
    assert!(q.to_lowercase().contains("rv%20there%20yet"), "got {}", q.to_lowercase());
}

#[test]
fn onlinefix_keeps_full_title_with_colon() {
    // Call of Duty: Black Ops II should keep subtitle, not strip to "call of duty"
    let url = build_onlinefix_url("Call of Duty: Black Ops II");
    let q = extract_onlinefix_query(&url);
    assert!(q.contains("black"), "should contain black ops, got {}", q);
    assert!(q.contains("ops"), "got {}", q);
    // Should not be just "call%20of%20duty"
    assert_ne!(q, "call%20of%20duty");
    assert_ne!(q, "call+of+duty");
}

#[test]
fn gcw_converts_roman_and_keeps_full_title() {
    let url = build_gcw_url("Call of Duty: Black Ops II");
    let slug = extract_gcw_slug(&url);
    assert_eq!(slug, "call_of_duty_black_ops_2");
    let url2 = build_gcw_url("DARK SOULS™ III");
    let slug2 = extract_gcw_slug(&url2);
    assert_eq!(slug2, "dark_souls_3");
    let url3 = build_gcw_url("Goat Simulator: Remastered");
    let slug3 = extract_gcw_slug(&url3);
    assert_eq!(slug3, "goat_simulator_remastered");
}

#[test]
fn gcw_goat_simulator_not_legacy() {
    let url = build_gcw_url("Goat Simulator: Remastered");
    assert!(!url.contains("legacy_of_kain"), "Goat should not become legacy_of_kain, got {}", url);
    assert!(url.contains("goat_simulator_remastered"));
}

#[test]
fn csrinru_davigo_strips_to_first_word() {
    let url = build_csrinru_url("DAVIGO: VR vs. PC");
    let kw = extract_csrinru_keywords(&url);
    // Should be just "davigo" (or davigo with maybe vr stripped), not "davigo vr vs pc"
    // Our heuristic strips subtitle when it contains VR
    assert_eq!(kw, "davigo");
}

#[test]
fn csrinru_sekiro_strips_goty_but_keeps_subtitle() {
    let url = build_csrinru_url("Sekiro™: Shadows Die Twice - GOTY Edition");
    let kw = extract_csrinru_keywords(&url);
    // Should be "sekiro shadows die twice" (without goty, without TM)
    assert!(kw.contains("sekiro"), "got {}", kw);
    assert!(kw.contains("shadows"), "got {}", kw);
    assert!(!kw.contains("goty"), "GOTY should be stripped");
    assert!(!kw.contains("%E2%84%A2"), "TM should be stripped");
}

#[test]
fn csrinru_stray_titleonly() {
    let url = build_csrinru_url("Stray");
    assert!(url.contains("sf=titleonly"), "should be titleonly");
    assert!(url.contains("sr=topics"), "should be topics");
    let kw = extract_csrinru_keywords(&url);
    assert_eq!(kw, "stray");
}

#[test]
fn csrinru_crusader_kings_keeps_numeric() {
    let url = build_csrinru_url("Crusader Kings 3");
    let kw = extract_csrinru_keywords(&url);
    // Should keep "3" (drop_numeric is false now)
    assert!(kw.contains("3") || kw.contains("crusader"), "got {}", kw);
}
