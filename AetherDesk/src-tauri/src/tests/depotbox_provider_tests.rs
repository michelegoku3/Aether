use std::io::Write;

use crate::manifest::package::ManifestPackageExtractor;
use crate::providers::depotbox::{
    classify_status, describe_api_error, parse_status, DepotBoxClient, JobPhase,
};
use crate::providers::manifest_source::ManifestSource;

/// One Lua pin plus its manifest, in the layout DepotBox packages use.
fn depotbox_zip() -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut cursor);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer
        .start_file("Half-Life/70.lua", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer
        .write_all(b"addappid(70)\nsetManifestid(70, \"1234567890123456789\")\n")
        .unwrap();
    writer
        .start_file(
            "Half-Life/depotcache/70_1234567890123456789.manifest",
            options,
        )
        .unwrap();
    writer.write_all(b"manifest-bytes").unwrap();
    writer.finish().unwrap();
    cursor.into_inner()
}

#[test]
fn manifest_source_metadata_is_stable() {
    let client = DepotBoxClient::new("test-key");
    assert_eq!(client.source_id(), "depotbox");
    assert_eq!(client.display_name(), "DepotBox");
}

#[test]
fn classifies_every_documented_job_state() {
    assert_eq!(classify_status("processing"), JobPhase::Processing);
    assert_eq!(classify_status("completed"), JobPhase::Completed);
    assert_eq!(
        classify_status("completed_with_warnings"),
        JobPhase::Warning
    );
    assert_eq!(classify_status("failed"), JobPhase::Failed);
    assert_eq!(classify_status("invalid_or_expired"), JobPhase::Expired);
    assert_eq!(classify_status("something_else"), JobPhase::Unknown);
}

#[test]
fn describes_http_failures_without_leaking_the_key() {
    let unauthorized = describe_api_error(401, "");
    assert!(unauthorized.to_lowercase().contains("key"));

    let forbidden = describe_api_error(403, r#"{"error":"Forbidden: Invalid API Key."}"#);
    assert!(forbidden.contains("Invalid API Key"));

    let missing = describe_api_error(404, "");
    assert!(missing.contains("404"));

    let limited = describe_api_error(
        429,
        r#"{"error":"Too Many Requests: API Key limit exceeded. Try again in 34 seconds."}"#,
    );
    assert!(limited.contains("34 seconds"));

    let server = describe_api_error(500, "");
    assert!(server.contains("500"));
}

#[test]
fn extracts_the_package_zip_output() {
    let package = ManifestPackageExtractor::from_zip(70, &depotbox_zip()).unwrap();
    assert!(package.lua_content.contains("setManifestid(70"));
    assert_eq!(package.manifest_files.len(), 1);
    assert_eq!(
        package.manifest_files[0].file_name,
        "70_1234567890123456789.manifest"
    );
    assert_eq!(package.manifest_files[0].bytes, b"manifest-bytes");
}

/// The documented shape is an array of strings, but the live service sends log
/// objects; that difference used to abort the whole download.
#[test]
fn parses_a_processing_status_whose_logs_are_objects() {
    let body = r#"{"status":"processing","message":"Fetching depot keys...","appId":"3618390","gameName":"Gamblers Table","logs":[{"timestamp":"2026-11-27T19:54:12Z","level":"info","message":"Fetching depot keys"},{"step":"packaging","detail":"Writing manifests"}]}"#;
    let status = parse_status(body).unwrap();
    assert_eq!(status.status, "processing");
    assert_eq!(status.message.as_deref(), Some("Fetching depot keys..."));
    assert_eq!(status.game_name.as_deref(), Some("Gamblers Table"));
    assert_eq!(
        status.logs,
        Some(vec![
            "Fetching depot keys".to_string(),
            "Writing manifests".to_string()
        ])
    );
}

#[test]
fn parses_log_entries_of_every_shape() {
    let body = r#"{"status":"processing","logs":["plain step",{"message":"object step"},42,{"unsupported":true}]}"#;
    let status = parse_status(body).unwrap();
    assert_eq!(
        status.logs,
        Some(vec![
            "plain step".to_string(),
            "object step".to_string(),
            "42".to_string(),
            r#"{"unsupported":true}"#.to_string()
        ])
    );
}

#[test]
fn parses_a_completed_status_with_a_zip_link() {
    let body = r#"{"status":"completed_with_warnings","message":"Done.","appId":"70","gameName":"Half-Life","logs":["Fetching depot keys...","Packaging manifests...","Done."],"finalUserZipName":"Half-Life-70.zip","isBatch":false,"download_link":"https://depotbox.lol/api/download/a1b2c3","expiry_minutes":"30"}"#;
    let status = parse_status(body).unwrap();
    assert_eq!(
        classify_status(&status.status),
        JobPhase::Warning,
        "a warnings payload must stay a terminal success"
    );
    assert_eq!(
        status.download_link.as_deref(),
        Some("https://depotbox.lol/api/download/a1b2c3")
    );
    assert_eq!(
        status.final_user_zip_name.as_deref(),
        Some("Half-Life-70.zip")
    );
    assert_eq!(status.expiry_minutes, Some(30));
}

#[test]
fn an_unreadable_status_names_the_payload_shape() {
    let error = parse_status(r#"{"success":true,"data":{"status":"completed"}}"#).unwrap_err();
    assert!(
        error.contains("status:string") || error.contains("success:bool"),
        "the error must describe the payload shape: {error}"
    );
    assert!(
        !error.contains("invalid type"),
        "raw serde wording must not reach the user: {error}"
    );

    let nested = parse_status(r#"{"status":{"state":"processing"}}"#).unwrap_err();
    assert!(
        nested.contains("status"),
        "a non-string state must be rejected: {nested}"
    );

    let broken = parse_status("not json").unwrap_err();
    assert!(broken.contains("unreadable status"), "got: {broken}");
}

#[test]
fn a_json_error_body_is_never_treated_as_a_package() {
    let error =
        ManifestPackageExtractor::from_zip(70, br#"{"error":"Forbidden: Invalid API Key."}"#)
            .unwrap_err();
    assert!(error.contains("manifest ZIP"));
}
