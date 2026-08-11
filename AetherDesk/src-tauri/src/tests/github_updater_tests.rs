//! Unit test per la logica di confronto versioni degli update (github.rs).
//!
//! Blinda il gate "la release test conta solo se piu' nuova dell'installata",
//! che e' la causa dei pallini rossi spuri quando l'utente e' gia' su una
//! versione piu' alta della release di test.

use crate::updater::github::GithubReleaseManager;

#[test]
fn test_release_never_older_or_equal_than_installed() {
    // Su 1.0.3, una release test 1.0.1 NON e' un update.
    assert!(!GithubReleaseManager::latest_is_newer_than("1.0.3", "tdesk-1.0.1"));
    assert!(!GithubReleaseManager::latest_is_newer_than("1.0.3", "desk-1.0.3"));
}

#[test]
fn test_test_release_newer_is_update() {
    // Su 1.0.3, una release test 1.0.4 E' un update (pallino rosso corretto).
    assert!(GithubReleaseManager::latest_is_newer_than("1.0.3", "tdesk-1.0.4"));
}

#[test]
fn test_stable_release_newer_is_update_after_test() {
    // Dopo aver installato la test 1.0.1, la stabile 1.0.3 e' un update.
    assert!(!GithubReleaseManager::latest_is_newer_than("1.0.1", "tdesk-1.0.1"));
    assert!(GithubReleaseManager::latest_is_newer_than("1.0.1", "desk-1.0.3"));
}

#[test]
fn test_component_version_strips_test_prefix() {
    assert_eq!(GithubReleaseManager::component_version_from_tag("tdesk-1.0.4"), "1.0.4");
    assert_eq!(GithubReleaseManager::component_version_from_tag("tdll-0.9.8"), "0.9.8");
    assert_eq!(GithubReleaseManager::component_version_from_tag("desk-1.0.3"), "1.0.3");
    assert_eq!(GithubReleaseManager::component_version_from_tag("dll-0.9.8"), "0.9.8");
}

#[test]
fn test_build_desk_test_update_info_respects_version_gate() {
    use crate::updater::github::GithubRelease;
    let release = GithubRelease {
        tag_name: "tdesk-1.0.4".to_string(),
        body: Some("Test notes".to_string()),
        html_url: Some("https://github.com/example/release".to_string()),
        assets: vec![],
    };

    // Quando la versione locale è uguale (1.0.4) o maggiore, update_available deve essere false
    let info_equal = GithubReleaseManager::build_desk_test_update_info("1.0.4".to_string(), &release);
    assert!(!info_equal.update_available);
    assert!(info_equal.is_test);

    // Quando la versione locale è minore (1.0.3), update_available deve essere true
    let info_older = GithubReleaseManager::build_desk_test_update_info("1.0.3".to_string(), &release);
    assert!(info_older.update_available);
    assert!(info_older.is_test);
}
