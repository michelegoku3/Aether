use std::future::Future;
use std::pin::Pin;

use crate::manifest::package::ManifestPackage;
use crate::providers::hubcap::HubcapClient;
use crate::providers::luatools::LuaToolsClient;
use crate::providers::oureveryday::OureverydayClient;
use crate::providers::ryuu::RyuuClient;

pub type SourceBoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// Unified trait implemented by all manifest and Lua download providers.
///
/// Decouples higher-level operations (such as installation, version preparation,
/// and backup coordination) from concrete provider implementations.
#[allow(dead_code)]
pub trait ManifestSource: Send + Sync {
    /// Machine slug identifying the provider (e.g. "hubcap", "ryuu", "luatools", "oureveryday").
    fn source_id(&self) -> &'static str;

    /// Human-friendly display label used in logs and notifications (e.g. "Hubcap", "Ryuu", "LuaTools", "MOED").
    fn display_name(&self) -> &'static str;

    /// Downloads the package containing the Lua file and any bundled binary manifests.
    fn download_package<'a>(&'a self, app_id: u32) -> SourceBoxFuture<'a, ManifestPackage>;
}

/// Helper function to dispatch a package download through any dynamic or static `ManifestSource`.
#[allow(dead_code)]
pub async fn download_package_from_source(
    source: &(dyn ManifestSource + '_),
    app_id: u32,
) -> Result<ManifestPackage, String> {
    source.download_package(app_id).await
}

impl ManifestSource for HubcapClient {
    fn source_id(&self) -> &'static str {
        "hubcap"
    }

    fn display_name(&self) -> &'static str {
        "Hubcap"
    }

    fn download_package<'a>(&'a self, app_id: u32) -> SourceBoxFuture<'a, ManifestPackage> {
        Box::pin(async move { self.download_lua_package(app_id).await })
    }
}

impl ManifestSource for RyuuClient {
    fn source_id(&self) -> &'static str {
        "ryuu"
    }

    fn display_name(&self) -> &'static str {
        "Ryuu"
    }

    fn download_package<'a>(&'a self, app_id: u32) -> SourceBoxFuture<'a, ManifestPackage> {
        Box::pin(async move { self.download_lua_package(app_id).await })
    }
}

impl ManifestSource for LuaToolsClient {
    fn source_id(&self) -> &'static str {
        "luatools"
    }

    fn display_name(&self) -> &'static str {
        "LuaTools"
    }

    fn download_package<'a>(&'a self, app_id: u32) -> SourceBoxFuture<'a, ManifestPackage> {
        Box::pin(async move { self.download_lua_package(app_id).await })
    }
}

impl ManifestSource for OureverydayClient {
    fn source_id(&self) -> &'static str {
        "oureveryday"
    }

    fn display_name(&self) -> &'static str {
        "MOED"
    }

    fn download_package<'a>(&'a self, app_id: u32) -> SourceBoxFuture<'a, ManifestPackage> {
        Box::pin(async move { self.download_lua_package(app_id).await })
    }
}
