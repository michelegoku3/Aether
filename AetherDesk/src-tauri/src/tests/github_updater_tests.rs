//! Unit tests for the update version-comparison logic (github.rs).
//!
//! Pins the "a test release only counts when newer than the installed one" gate,
//! the cause of spurious red dots when the user is already on a version higher
//! than the test release.

use crate::updater::github::GithubReleaseManager;

#[test]
fn test_release_never_older_or_equal_than_installed() {
    // On 1.0.3, a test release 1.0.1 is NOT an update.
    assert!(!GithubReleaseManager::latest_is_newer_than(
        "1.0.3",
        "tdesk-1.0.1"
    ));
    assert!(!GithubReleaseManager::latest_is_newer_than(
        "1.0.3",
        "desk-1.0.3"
    ));
}

#[test]
fn test_test_release_newer_is_update() {
    // On 1.0.3, a test release 1.0.4 IS an update (correct red dot).
    assert!(GithubReleaseManager::latest_is_newer_than(
        "1.0.3",
        "tdesk-1.0.4"
    ));
}

#[test]
fn test_stable_release_newer_is_update_after_test() {
    // After installing test 1.0.1, stable 1.0.3 is an update.
    assert!(!GithubReleaseManager::latest_is_newer_than(
        "1.0.1",
        "tdesk-1.0.1"
    ));
    assert!(GithubReleaseManager::latest_is_newer_than(
        "1.0.1",
        "desk-1.0.3"
    ));
}

#[test]
fn test_component_version_strips_test_prefix() {
    assert_eq!(
        GithubReleaseManager::component_version_from_tag("tdesk-1.0.4"),
        "1.0.4"
    );
    assert_eq!(
        GithubReleaseManager::component_version_from_tag("tdll-0.9.8"),
        "0.9.8"
    );
    assert_eq!(
        GithubReleaseManager::component_version_from_tag("desk-1.0.3"),
        "1.0.3"
    );
    assert_eq!(
        GithubReleaseManager::component_version_from_tag("dll-0.9.8"),
        "0.9.8"
    );
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

    // When the local version is equal (1.0.4) or newer, update_available must be false
    let info_equal =
        GithubReleaseManager::build_desk_test_update_info("1.0.4".to_string(), &release);
    assert!(!info_equal.update_available);
    assert!(info_equal.is_test);

    // When the local version is older (1.0.3), update_available must be true
    let info_older =
        GithubReleaseManager::build_desk_test_update_info("1.0.3".to_string(), &release);
    assert!(info_older.update_available);
    assert!(info_older.is_test);
}

// ============================================================================
// F6/F7 — channel absence is not an error, and lookups are deduplicated
// ============================================================================

#[test]
fn channel_absence_is_classified_separately_from_a_failed_lookup() {
    use crate::updater::github::channel_has_no_release;

    // Probing an unpublished test channel is a normal state, not a fault.
    assert!(channel_has_no_release(
        "No published GitHub release found for prefixes [\"tdll-\", \"tdll-v\"]"
    ));
    assert!(channel_has_no_release(
        "No release atom entry found for prefixes [\"tdesk-\"]"
    ));

    // Real failures stay failures.
    assert!(!channel_has_no_release(
        "GitHub API network error: connection reset by peer"
    ));
    assert!(!channel_has_no_release(
        "GitHub API request failed with status 403: rate limit exceeded"
    ));
}

#[test]
fn completed_lookup_is_reused_instead_of_refetching() {
    use crate::updater::github::{fresh_for_tests, lookup_slot_for_tests, publish_for_tests};

    let release = crate::updater::github::GithubRelease {
        tag_name: "dll-1.2.3".to_string(),
        body: None,
        html_url: None,
        draft: false,
        prerelease: false,
        assets: Vec::new(),
    };

    let slot = lookup_slot_for_tests("test-channel-reuse");
    assert!(
        fresh_for_tests(&slot).is_none(),
        "a fresh slot has nothing to serve"
    );

    publish_for_tests(&slot, Ok(release));
    let cached = fresh_for_tests(&slot).expect("published result is reusable");
    assert_eq!(cached.expect("ok").tag_name, "dll-1.2.3");

    // The same key always resolves to the same slot: the second IPC call of a
    // settings save joins the first instead of hitting the GitHub API again.
    let same = lookup_slot_for_tests("test-channel-reuse");
    assert!(fresh_for_tests(&same).is_some());
}

#[test]
fn an_empty_channel_is_also_remembered() {
    use crate::updater::github::{fresh_for_tests, lookup_slot_for_tests, publish_for_tests};

    let slot = lookup_slot_for_tests("test-channel-empty");
    publish_for_tests(
        &slot,
        Err("No published GitHub release found for prefixes [\"tdll-\"]".to_string()),
    );

    // Deduplicating only the success path would move the retry storm to the
    // error path: an empty channel is cached too (for longer, since a release
    // only appears on release day).
    let cached = fresh_for_tests(&slot).expect("the absence is remembered");
    assert!(cached.is_err());
}

#[test]
fn test_etag_header_parsing() {
    let header_val = "\"64f8a3c8e9d1\"";
    let parsed = header_val.trim_matches('"');
    assert_eq!(parsed, "64f8a3c8e9d1");
}

#[tokio::test]
async fn a_stalled_body_aborts_after_the_idle_timeout() {
    use std::time::{Duration, Instant};
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n")
            .await;
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let client = crate::updater::http::update_client_with(
        Duration::from_secs(2),
        Duration::from_millis(500),
    );
    let started = Instant::now();
    let response = client
        .get(format!("http://{addr}/stalled"))
        .send()
        .await
        .expect("headers arrive");
    let error = response
        .bytes()
        .await
        .expect_err("the body must be cut off");
    assert!(error.is_timeout(), "unexpected error: {error}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_slow_but_alive_body_is_not_aborted() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        // Drain the request before answering: closing with unread bytes makes
        // Windows answer with RST, which the client would see as a reset.
        let mut request = [0u8; 1024];
        let _ = socket.read(&mut request).await;
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 300\r\n\r\n")
            .await;
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = socket.write_all(&[0u8; 100]).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    });

    let client = crate::updater::http::update_client_with(
        Duration::from_secs(2),
        Duration::from_millis(500),
    );
    let response = client
        .get(format!("http://{addr}/slow"))
        .send()
        .await
        .expect("headers arrive");
    let body = response.bytes().await.expect("a slow body still completes");
    assert_eq!(body.len(), 300);
}

/// Serves one fixed body and returns its URL. The request is drained first so
/// the socket closes with FIN instead of a Windows RST.
async fn serve_once(body: &'static [u8], content_type: &'static str) -> String {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = [0u8; 1024];
        let _ = socket.read(&mut request).await;
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(header.as_bytes()).await;
        let _ = socket.write_all(body).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    });
    format!("http://{addr}/file")
}

fn updater_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aether_update_dl_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[tokio::test]
async fn download_streams_to_disk_and_reports_monotonic_progress() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // 3 chunks of 100 bytes, one per write, so `chunk()` yields several times.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = [0u8; 1024];
        let _ = socket.read(&mut request).await;
        let _ = socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: 300\r\n\r\n",
            )
            .await;
        for _ in 0..3 {
            let _ = socket.write_all(&[7u8; 100]).await;
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
    });

    let dir = updater_temp_dir("progress");
    let dest = dir.join("update.zip");
    let mut seen: Vec<(u64, Option<u64>)> = Vec::new();
    {
        let mut on_progress = |received: u64, total: Option<u64>| seen.push((received, total));
        crate::updater::desk::download_zip(&format!("http://{addr}/file"), &dest, &mut on_progress)
            .await
            .expect("download succeeds");
    }

    assert_eq!(std::fs::read(&dest).expect("file written").len(), 300);
    assert_eq!(seen.last(), Some(&(300, Some(300))));
    assert!(
        seen.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "progress must not go backwards: {seen:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_200_that_is_html_is_refused_before_extraction() {
    let url = serve_once(b"<html>captive portal</html>", "text/html").await;
    let dir = updater_temp_dir("html");
    let dest = dir.join("update.zip");

    let error = crate::updater::desk::download_zip(&url, &dest, &mut |_, _| {})
        .await
        .expect_err("an html page is not an update");

    assert!(error.contains("not an update ZIP"), "unexpected: {error}");
    assert!(!dest.exists(), "nothing may be published at the final path");
    assert!(
        !dest.with_extension("tmp").exists(),
        "temp file left behind"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_failed_write_leaves_no_file_at_the_final_path() {
    let url = serve_once(b"PK\x03\x04payload", "application/zip").await;
    let dir = updater_temp_dir("writefail");
    let dest = dir.join("update.zip");
    // A directory squatting on the temp name makes the write fail.
    std::fs::create_dir_all(dest.with_extension("tmp")).expect("blocking dir");

    let error = crate::updater::desk::download_zip(&url, &dest, &mut |_, _| {})
        .await
        .expect_err("the write must fail");

    assert!(!error.is_empty());
    assert!(!dest.exists(), "no partial file at the final path");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_second_update_claim_is_refused_until_the_first_releases() {
    use crate::updater::desk::{claim_update_slot, release_update_slot};

    assert!(claim_update_slot());
    assert!(!claim_update_slot(), "a second click must be refused");
    release_update_slot();
    assert!(claim_update_slot(), "the slot is reusable after release");
    release_update_slot();
}

/// The payload's `phase` values are a shared contract with
/// `src/constants/updates.ts`: renaming one here without renaming it there
/// would leave the UI stuck on an unknown phase.
#[test]
fn update_phase_wire_names_match_the_frontend_union() {
    use crate::updater::desk::UpdatePhase;

    let names: Vec<&str> = [
        UpdatePhase::LookingUp,
        UpdatePhase::Downloading,
        UpdatePhase::Extracting,
        UpdatePhase::Staging,
        UpdatePhase::Restarting,
    ]
    .iter()
    .map(|phase| phase.wire_name())
    .collect();

    assert_eq!(
        names,
        vec![
            "looking-up",
            "downloading",
            "extracting",
            "staging",
            "restarting"
        ]
    );
}
