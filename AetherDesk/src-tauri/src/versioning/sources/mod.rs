pub mod depotbox;
pub mod steamdb;

use std::future::Future;
use std::pin::Pin;

use crate::manifest::pins::DepotManifestPin;
use crate::versioning::error::VersionError;
use crate::versioning::model::BuildInfo;

/// Owned boxed future, so source traits stay object-safe without pulling in
/// an async-trait dependency.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Provides a game's build history (`{date, build_id}` entries).
pub trait BuildHistorySource: Send + Sync {
    fn build_history(&self, app_id: u32) -> BoxFuture<'_, Result<Vec<BuildInfo>, VersionError>>;
}

/// Resolves a BuildID into the (depot, manifest) pins of that build.
pub trait BuildDetailsSource: Send + Sync {
    fn pins_for_build(&self, build_id: u64) -> BoxFuture<'_, Result<Vec<DepotManifestPin>, VersionError>>;
}

// ── Build-details access token ───────────────────────────────────────────
//
// The Depotbox build-details endpoint requires an `x-api-key` header. The key
// below is the one SFF ships inside its public, GPL-licensed source
// (github.com/Midrags/SFF, `lua/endpoints.py`); it is embedded as a default so
// Build lookups work out of the box exactly like they do for every SFF user.
// It is NOT a documented public API and may be rotated or revoked at any
// time, so it can always be overridden with the `AETHERDESK_BUILD_TOKEN`
// environment variable or the `build_details_token` setting.
pub const DEFAULT_BUILD_DETAILS_TOKEN: &str = "dbxpriv_7785b0eca2c32385830332832ed8443539ab4f5b084779f7";

/// Resolution order: env override → user setting → built-in default.
pub fn resolve_build_details_token(settings_token: Option<&str>) -> Option<String> {
    if let Ok(env_value) = std::env::var("AETHERDESK_BUILD_TOKEN") {
        let env_value = env_value.trim().to_string();
        if !env_value.is_empty() {
            return Some(env_value);
        }
    }
    if let Some(setting) = settings_token {
        let setting = setting.trim().to_string();
        if !setting.is_empty() {
            return Some(setting);
        }
    }
    Some(DEFAULT_BUILD_DETAILS_TOKEN.to_string())
}
