use std::time::Duration;
use crate::manifest::package::{ManifestPackage, ManifestPackageFile, ManifestPackageExtractor};
use crate::manifest::pins::LuaManifestPins;
use crate::providers::http;

const LUA_GEN_TIMEOUT_SECONDS: u64 = 15;
const MANIFEST_MIRROR_TIMEOUT_SECONDS: u64 = 10;

const GITHUB_MANIFEST_REPOS: &[(&str, &str)] = &[
    ("qwe213312/k25FCdfEOoEJ42S6", "qwe213312"),
    ("mejikuhibiniu1/k25FCdfEOoEJ42S6", "mejikuhibiniu1"),
    ("Sainan/k25FCdfEOoEJ42S6", "Sainan"),
];

pub struct OureverydayClient {
    client: reqwest::Client,
}

impl OureverydayClient {
    pub fn new() -> Self {
        Self {
            client: http::build_client(LUA_GEN_TIMEOUT_SECONDS),
        }
    }

    /// Downloads the public Lua file by App ID from the luagen API (revobd.club)
    /// and extracts it.
    pub async fn download_lua_only(&self, app_id: u32) -> Result<String, String> {
        crate::desk_log_info!("oureveryday", "Requesting public Lua package for {} from https://api.luagen.revobd.club/{}.zip", crate::core::logger::format_appid(app_id), app_id);
        let url = format!("https://api.luagen.revobd.club/{}.zip", app_id);
        
        let response = self.client.get(&url)
            .send()
            .await
            .map_err(|e| {
                crate::desk_log_error!("oureveryday", "Network error connecting to luagen server for {}: {}", crate::core::logger::format_appid(app_id), e);
                format!("Network error connecting to luagen server: {}", e)
            })?;

        if !response.status().is_success() {
            crate::desk_log_error!("oureveryday", "The public luagen server returned HTTP status {} for {}", response.status(), crate::core::logger::format_appid(app_id));
            return Err(format!("The public luagen server returned HTTP status: {}", response.status()));
        }

        let bytes = response.bytes().await
            .map_err(|e| {
                crate::desk_log_error!("oureveryday", "Failed to read luagen ZIP bytes for {}: {}", crate::core::logger::format_appid(app_id), e);
                format!("Failed to read luagen ZIP bytes: {}", e)
            })?;

        let package = match ManifestPackageExtractor::from_zip(app_id, &bytes) {
            Ok(p) => p,
            Err(e) => {
                crate::desk_log_error!("oureveryday", "Failed to extract package from luagen ZIP for {}: {}", crate::core::logger::format_appid(app_id), e);
                return Err(e);
            }
        };
        crate::desk_log_info!("oureveryday", "Downloaded public Lua package for {} successfully", crate::core::logger::format_appid(app_id));
        Ok(package.lua_content)
    }

    /// Downloads the single manifest file from the public raw GitHub mirrors.
    /// Tries each mirror in sequence until one succeeds.
    pub async fn download_manifest_file(&self, depot_id: u32, manifest_id: &str) -> Result<Vec<u8>, String> {
        let mut last_err = String::from("No mirrors configured");

        for &(repo, label) in GITHUB_MANIFEST_REPOS {
            let url = format!(
                "https://raw.githubusercontent.com/{}/main/{}_{}.manifest",
                repo, depot_id, manifest_id
            );

            match self.client.get(&url)
                .timeout(Duration::from_secs(MANIFEST_MIRROR_TIMEOUT_SECONDS))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.bytes().await {
                            Ok(bytes) => return Ok(bytes.to_vec()),
                            Err(e) => {
                                last_err = format!("Failed to read bytes from mirror ({}): {}", label, e);
                            }
                        }
                    } else if response.status().as_u16() == 404 {
                        last_err = format!("Manifest not found on mirror ({}) - 404", label);
                    } else {
                        last_err = format!("Mirror ({}) returned HTTP status: {}", label, response.status());
                    }
                }
                Err(e) => {
                    last_err = format!("Failed to connect to mirror ({}): {}", label, e);
                }
            }
        }

        crate::desk_log_error!("oureveryday", "Failed to download manifest {}_{}.manifest from all public GitHub mirrors: {}", depot_id, manifest_id, last_err);
        Err(last_err)
    }

    /// Fetches the Lua file from revobd, parses the manifest IDs inside,
    /// downloads all corresponding manifest files from public GitHub mirrors in parallel,
    /// and packages them together into a Unified ManifestPackage.
    pub async fn download_lua_package(&self, app_id: u32) -> Result<ManifestPackage, String> {
        // 1. Download and extract the main Lua file
        let lua_content = self.download_lua_only(app_id).await?;

        // 2. Parse out setManifestid depot/manifest rows
        let rows = LuaManifestPins::rows_from_content(&lua_content);

        // 3. Download the manifests in parallel (leveraging tokio tasks)
        let mut manifest_futures = Vec::new();
        for row in rows {
            let depot_id = row.app_id; // in LuaManifestRow, app_id holds the depot ID
            let manifest_id = row.manifest_id.clone();
            
            let client_clone = http::build_client(MANIFEST_MIRROR_TIMEOUT_SECONDS);
            
            // Spawn parallel tasks to download from the mirrors
            manifest_futures.push(tokio::spawn(async move {
                let oe = OureverydayClient { client: client_clone };
                match oe.download_manifest_file(depot_id, &manifest_id).await {
                    Ok(bytes) => Some(ManifestPackageFile {
                        file_name: format!("{}_{}.manifest", depot_id, manifest_id),
                        bytes,
                    }),
                    Err(_) => {
                        // Silent fallback - if one fails, we just don't add it to the package,
                        // allowing the Lua setup to proceed even if some manifest isn't in mirrors.
                        None
                    }
                }
            }));
        }

        // Wait for all manifest downloads to finish
        let mut manifest_files = Vec::new();
        for future in manifest_futures {
            if let Ok(Some(file)) = future.await {
                manifest_files.push(file);
            }
        }

        crate::desk_log_info!("oureveryday", "Successfully packaged {} with {} manifest file(s) from Oureveryday", crate::core::logger::format_appid(app_id), manifest_files.len());
        Ok(ManifestPackage {
            lua_content,
            manifest_files,
        })
    }
}
