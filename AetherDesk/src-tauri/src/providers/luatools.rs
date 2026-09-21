use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::manifest::identity::verify_manifest_bytes;
use crate::manifest::package::{ManifestPackage, ManifestPackageExtractor, ManifestPackageFile};
use crate::manifest::pins::DepotManifestPin;
use crate::manifest::resolver::manifest_file_name;
use crate::providers::http;
use crate::providers::luatools_auth::LuaToolsAuth;

const API_BASE_URL: &str = "https://lua.tools";
const DOWNLOAD_TIMEOUT_SECONDS: u64 = 5 * 60;
/// Per-request budget of the per-depot manifest endpoint: one manifest is a
/// few hundred KB at most, and the calls run concurrently.
const DEPOT_MANIFEST_TIMEOUT_SECONDS: u64 = 60;
/// Concurrent per-depot manifest requests. lua.tools allows 120 of these per
/// 10 minutes per user (cache misses only); a large game is ~20 depots, so a
/// small fan-out keeps a full completion well under a minute without ever
/// looking like a burst.
const DEPOT_MANIFEST_CONCURRENCY: usize = 3;

/// LuaTools is an authenticated source aggregator. Availability is discovered
/// from its manifest backend; downloads use the user's own LuaTools Supabase
/// session and therefore count against that account's server-side allowance.
#[derive(Clone)]
pub struct LuaToolsClient {
    auth: Arc<LuaToolsAuth>,
    client: reqwest::Client,
}

/// Outcome of [`LuaToolsClient::complete_manifests`].
#[derive(Debug, Default)]
pub struct LuaToolsCompletion {
    /// Verified `<depot>_<gid>.manifest` files fetched from LuaTools, ready to
    /// be installed together with the Lua.
    pub fetched: Vec<ManifestPackageFile>,
    /// Pins LuaTools could not serve, with the provider's reason. Callers
    /// decide whether another provider may complete them.
    pub failed: Vec<(DepotManifestPin, String)>,
}

impl LuaToolsClient {
    pub fn new() -> Self {
        Self {
            auth: Arc::new(LuaToolsAuth::new()),
            client: http::build_client(DOWNLOAD_TIMEOUT_SECONDS),
        }
    }

    /// Sources lua.tools currently reports as `available` for `app_id`, best
    /// first (see [`rank_available_sources`]). Errors when there is none.
    pub async fn available_sources(&self, app_id: u32) -> Result<Vec<String>, String> {
        let access_token = self.auth.valid_access_token().await?;
        let statuses = self.source_statuses(app_id, &access_token).await?;
        let ranked = rank_available_sources(&statuses);
        if ranked.is_empty() {
            return Err(format!("LuaTools has no available source for App ID {app_id}"));
        }
        Ok(ranked)
    }

    /// Package from the best available source (see
    /// [`Self::download_lua_package_from`] for the wire contract).
    pub async fn download_lua_package(
        &self,
        app_id: u32,
        game_name: Option<&str>,
    ) -> Result<ManifestPackage, String> {
        let sources = self.available_sources(app_id).await?;
        self.download_lua_package_from(app_id, &sources[0], game_name).await
    }

    /// `GET /api/manifest/download?appid=…&source=…[&game_name=…]`
    ///
    /// The endpoint answers with `<appid>.zip` (Lua + `.manifest` files) for
    /// the sources that archive manifests, or with a bare pinned Lua for the
    /// ones that only mirror the entitlement file (e.g. Luie). The payload
    /// bytes decide the format, never the filename. `game_name` is the same
    /// optional hint the official client sends: it only feeds the account's
    /// download history on the server, so it is passed when known and skipped
    /// otherwise. Every call counts against the account's daily allowance.
    pub async fn download_lua_package_from(
        &self,
        app_id: u32,
        source: &str,
        game_name: Option<&str>,
    ) -> Result<ManifestPackage, String> {
        let access_token = self.auth.valid_access_token().await?;
        let mut query: Vec<(&str, String)> = vec![
            ("appid", app_id.to_string()),
            ("source", source.to_string()),
        ];
        if let Some(name) = game_name.map(str::trim).filter(|name| !name.is_empty()) {
            query.push(("game_name", name.chars().take(200).collect()));
        }
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/manifest/download"))
            .bearer_auth(access_token)
            .query(&query)
            .send()
            .await
            .map_err(|e| format!("LuaTools download failed: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "LuaTools source {source} returned HTTP {status}: {}",
                describe_api_error(status, &detail)
            ));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Could not read the LuaTools download: {e}"))?;
        crate::desk_log_info!(
            "luatools",
            "Downloaded LuaTools package for {} from source '{}' ({} bytes, content-type={})",
            crate::core::logger::format_appid(app_id),
            source,
            bytes.len(),
            content_type
        );
        ManifestPackageExtractor::from_provider_bytes(app_id, bytes.as_ref())
    }

    /// `GET /api/givemethemanifestpunk/{depot_id}/{manifest_id}`
    ///
    /// One depot's raw `.manifest` by id. Same Bearer session as the package
    /// download but a different budget on the server: it writes no history
    /// row and does not consume the 25/day package allowance (it is limited
    /// to 120 requests per 10 minutes per user, cache misses only). The body
    /// is raw manifest bytes on 200 — occasionally wrapped in a ZIP with a
    /// single entry — and a JSON error otherwise. The bytes are verified
    /// against the requested depot/GID before they are returned, so a wrong
    /// or corrupt file can never be published into depotcache.
    ///
    /// Takes the access token from the caller: [`Self::complete_manifests`]
    /// runs several of these concurrently and they must share ONE session
    /// check/refresh (Supabase refresh tokens are single-use, so racing
    /// refreshes would sign the user out).
    async fn download_depot_manifest(
        &self,
        pin: &DepotManifestPin,
        access_token: &str,
    ) -> Result<Vec<u8>, String> {
        let manifest_gid: u64 = pin
            .manifest_id
            .trim()
            .parse()
            .map_err(|_| format!("Invalid manifest id '{}' for depot {}", pin.manifest_id, pin.depot_id))?;
        if pin.depot_id == 0 || manifest_gid == 0 {
            return Err("Depot manifest fetch requires a valid depot ID and manifest ID.".to_string());
        }
        let response = self
            .client
            .get(format!(
                "{API_BASE_URL}/api/givemethemanifestpunk/{}/{}",
                pin.depot_id, manifest_gid
            ))
            .bearer_auth(access_token)
            .timeout(Duration::from_secs(DEPOT_MANIFEST_TIMEOUT_SECONDS))
            .send()
            .await
            .map_err(|e| format!("LuaTools manifest request failed: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "LuaTools returned HTTP {status} for manifest {}_{}: {}",
                pin.depot_id,
                manifest_gid,
                describe_api_error(status, &detail)
            ));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Could not read LuaTools manifest {}_{}: {e}", pin.depot_id, manifest_gid))?;
        if bytes.is_empty() {
            return Err(format!(
                "LuaTools returned an empty body for manifest {}_{}",
                pin.depot_id, manifest_gid
            ));
        }
        verify_manifest_bytes(bytes.as_ref(), pin.depot_id, manifest_gid)
            .map_err(|error| format!("LuaTools manifest {}_{}: {error}", pin.depot_id, manifest_gid))
    }

    /// Fetches the given pins one by one through the per-depot endpoint
    /// (bounded concurrency) and returns the verified files plus the pins that
    /// could not be served. Never fails as a whole: a single poisoned depot
    /// must not hide the manifests that did arrive.
    pub async fn complete_manifests(
        &self,
        app_id: u32,
        pins: &[DepotManifestPin],
    ) -> LuaToolsCompletion {
        let mut completion = LuaToolsCompletion::default();
        if pins.is_empty() {
            return completion;
        }
        crate::desk_log_info!(
            "luatools",
            "Completing {} manifest(s) for {} through the LuaTools per-depot endpoint",
            pins.len(),
            crate::core::logger::format_appid(app_id)
        );
        // One session check/refresh for the whole batch (see
        // `download_depot_manifest`).
        let access_token = match self.auth.valid_access_token().await {
            Ok(token) => Arc::new(token),
            Err(error) => {
                completion.failed = pins.iter().cloned().map(|pin| (pin, error.clone())).collect();
                return completion;
            }
        };
        for wave in pins.chunks(DEPOT_MANIFEST_CONCURRENCY) {
            let mut tasks = tokio::task::JoinSet::new();
            for pin in wave {
                let client = self.clone();
                let pin = pin.clone();
                let access_token = Arc::clone(&access_token);
                tasks.spawn(async move {
                    let result = client
                        .download_depot_manifest(&pin, access_token.as_str())
                        .await;
                    (pin, result)
                });
            }
            while let Some(joined) = tasks.join_next().await {
                let (pin, result) = match joined {
                    Ok(pair) => pair,
                    Err(error) => {
                        crate::desk_log_warn!(
                            "luatools",
                            "Depot manifest task failed to run for {}: {}",
                            crate::core::logger::format_appid(app_id),
                            error
                        );
                        continue;
                    }
                };
                match result {
                    Ok(bytes) => {
                        crate::desk_log_info!(
                            "luatools",
                            "Fetched manifest {}_{} for {} ({} bytes)",
                            pin.depot_id,
                            pin.manifest_id,
                            crate::core::logger::format_appid(app_id),
                            bytes.len()
                        );
                        completion.fetched.push(ManifestPackageFile {
                            file_name: manifest_file_name(&pin),
                            bytes,
                        });
                    }
                    Err(error) => {
                        crate::desk_log_warn!(
                            "luatools",
                            "Manifest {}_{} for {} not served by LuaTools: {}",
                            pin.depot_id,
                            pin.manifest_id,
                            crate::core::logger::format_appid(app_id),
                            error
                        );
                        completion.failed.push((pin, error));
                    }
                }
            }
        }
        // Deterministic order for logs, backups and the install step.
        completion
            .fetched
            .sort_by(|a, b| a.file_name.cmp(&b.file_name));
        completion
            .failed
            .sort_by(|(a, _), (b, _)| (a.depot_id, &a.manifest_id).cmp(&(b.depot_id, &b.manifest_id)));
        completion
    }

    /// `GET /api/manifest/check?appid=…` → `{ "<source>": "<status>", … }`.
    async fn source_statuses(
        &self,
        app_id: u32,
        access_token: &str,
    ) -> Result<HashMap<String, String>, String> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/manifest/check"))
            .bearer_auth(access_token)
            .query(&[("appid", app_id)])
            .send()
            .await
            .map_err(|e| format!("Could not check LuaTools sources: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "LuaTools source check returned HTTP {}",
                response.status()
            ));
        }
        response
            .json()
            .await
            .map_err(|e| format!("Could not parse LuaTools source availability: {e}"))
    }
}

/// Turns a lua.tools error body (`{"error": "..."}` or plain text) into the
/// short human-readable detail the UI shows. The 401/429 wording mirrors the
/// official client so users recognise the situation.
pub(crate) fn describe_api_error(status: reqwest::StatusCode, body: &str) -> String {
    let server_message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .filter(|message| !message.trim().is_empty());
    match status.as_u16() {
        401 => "login required — sign in to LuaTools again from Settings".to_string(),
        429 => server_message.unwrap_or_else(|| {
            "rate limit or daily download limit reached (25/day)".to_string()
        }),
        _ => server_message.unwrap_or_else(|| {
            body.chars()
                .take(160)
                .map(|character| if character.is_control() { ' ' } else { character })
                .collect::<String>()
                .trim()
                .to_string()
        }),
    }
}

/// Every `available` source, best first. A stable preference keeps automatic
/// selection deterministic even though JSON object order is not part of the
/// manifest backend contract.
///
/// The order is "ships its own manifests" first: Ryuu / Sushi / Skyflare /
/// TwentyTwo Cloud answer with `<appid>.zip` (Lua + `.manifest` files), so the
/// package is complete on arrival. Luie — lua.tools' own Discord-bot source —
/// answers with a bare Lua, which is only as good as what the per-depot
/// completion can fetch afterwards; since September 2026 Steam no longer hands
/// out manifests for apps the account does not own, so a package that already
/// contains them is the more reliable install. Unknown names follow the known
/// ones, alphabetically.
pub(crate) fn rank_available_sources(statuses: &HashMap<String, String>) -> Vec<String> {
    const PREFERENCE: &[&str] = &["Ryuu", "Sushi", "Skyflare", "TwentyTwo Cloud", "Luie"];
    let is_available = |status: &String| status.eq_ignore_ascii_case("available");
    let mut ranked: Vec<String> = Vec::new();
    for preferred in PREFERENCE {
        if let Some((name, _)) = statuses
            .iter()
            .find(|(name, status)| name.eq_ignore_ascii_case(preferred) && is_available(status))
        {
            ranked.push(name.clone());
        }
    }
    let mut others: Vec<String> = statuses
        .iter()
        .filter(|(name, status)| {
            is_available(status) && !PREFERENCE.iter().any(|known| name.eq_ignore_ascii_case(known))
        })
        .map(|(name, _)| name.clone())
        .collect();
    others.sort();
    ranked.extend(others);
    ranked
}

/// First entry of [`rank_available_sources`].
pub(crate) fn choose_available_source(statuses: &HashMap<String, String>) -> Option<String> {
    rank_available_sources(statuses).into_iter().next()
}
