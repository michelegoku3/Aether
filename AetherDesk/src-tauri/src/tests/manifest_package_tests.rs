use crate::manifest::package::ManifestPackageExtractor;

#[test]
fn provider_bytes_accepts_bare_pinned_lua() {
    let lua = b"addappid(1158311, 1, \"key\")\nsetManifestid(1158311, \"123456789\")\n";

    let package = ManifestPackageExtractor::from_provider_bytes(1158310, lua)
        .expect("bare Lua should be accepted");

    assert!(package.lua_content.contains("setManifestid(1158311"));
    assert!(package.manifest_files.is_empty());
}

#[test]
fn provider_bytes_accepts_utf8_bom_before_lua() {
    let lua = "\u{feff}addappid(1)\nsetManifestid(2, \"3\")\n";

    let package = ManifestPackageExtractor::from_provider_bytes(1, lua.as_bytes())
        .expect("BOM-prefixed Lua should be accepted");

    assert!(package.lua_content.starts_with("addappid"));
}

#[test]
fn provider_bytes_rejects_successful_json_or_html_responses() {
    let error = ManifestPackageExtractor::from_provider_bytes(
        1,
        br#"{"error":"daily limit reached"}"#,
    )
    .expect_err("JSON must not be installed as Lua");

    assert!(error.contains("neither a manifest ZIP nor a pinned Lua file"));
    assert!(error.contains("daily limit reached"));
}
