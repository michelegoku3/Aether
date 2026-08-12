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
fn test_atom_and_html_tag_parsing() {
    let atom = r#"
        <feed>
          <entry>
            <link rel="alternate" href="https://github.com/michelegoku3/Aether/releases/tag/desk-1.0.4"/>
          </entry>
          <entry>
            <id>tag:github.com,2008:Repository/1305584107/dll-0.9.8</id>
            <link href="https://github.com/michelegoku3/Aether/releases/tag/dll-0.9.8"/>
          </entry>
        </feed>
    "#;
    let tags = GithubReleaseManager::release_tags_from_atom(atom);
    assert!(tags.contains(&"desk-1.0.4".to_string()));
    assert!(tags.contains(&"dll-0.9.8".to_string()));

    let html = r#"<a href="/michelegoku3/Aether/releases/tag/tdesk-1.0.5">tdesk-1.0.5</a>"#;
    let html_tags = GithubReleaseManager::release_tags_from_atom(html);
    assert_eq!(html_tags, vec!["tdesk-1.0.5".to_string()]);
}

#[test]
fn test_conventional_download_urls_do_not_use_api() {
    let desk = GithubReleaseManager::conventional_assets("desk-1.0.4");
    assert!(desk.iter().any(|asset| {
        asset.name == "AetherDesk-1.0.4.zip"
            && asset.browser_download_url
                == "https://github.com/michelegoku3/Aether/releases/download/desk-1.0.4/AetherDesk-1.0.4.zip"
    }));

    let dll = GithubReleaseManager::conventional_assets("dll-0.9.8");
    assert!(dll.iter().any(|asset| {
        asset.name == "AetherDLL-0.9.8.zip"
            && asset.browser_download_url
                == "https://github.com/michelegoku3/Aether/releases/download/dll-0.9.8/AetherDLL-0.9.8.zip"
    }));
}

#[test]
fn test_build_desk_test_update_info_respects_version_gate() {
    use crate::updater::github::GithubRelease;
    let release = GithubRelease {
        tag_name: "tdesk-1.0.4".to_string(),
        body: Some("Test notes".to_string()),
        html_url: Some("https://github.com/example/release".to_string()),
        draft: false,
        prerelease: false,
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
