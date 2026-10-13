//! Snapshot of the Tauri WebView security policy (`tauri.conf.json`).
//!
//! The CSP is a deliberate security control (see SEC-002): themes are injected
//! as `<style>` (hence `style-src 'unsafe-inline'`), covers come from Steam
//! CDNs plus wallpaper data-URIs (hence `img-src https: data:`), and no
//! remote scripts are ever needed. Any intentional change to this string must
//! update this snapshot AND get a security review first — that friction is
//! the point of the test.

const EXPECTED_CSP: &str = "default-src 'self'; img-src 'self' https: data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self' https:; font-src 'self' data:";

#[test]
fn webview_csp_matches_the_reviewed_snapshot() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(manifest_dir).join("tauri.conf.json");
    let content = std::fs::read_to_string(&path)
        .expect("tauri.conf.json must be readable next to Cargo.toml");
    let config: serde_json::Value =
        serde_json::from_str(&content).expect("tauri.conf.json must be valid JSON");
    let csp = config
        .pointer("/app/security/csp")
        .and_then(|value| value.as_str())
        .expect("app.security.csp must be a string (never null: no policy is not a policy)");

    assert_eq!(
        csp, EXPECTED_CSP,
        "WebView CSP changed: update EXPECTED_CSP here AND review the security impact first (themes need style 'unsafe-inline', covers need img https:/data:, scripts must stay 'self'-only)"
    );
}
