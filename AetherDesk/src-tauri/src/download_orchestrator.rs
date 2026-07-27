use crate::hubcap_client::HubcapClient;
use crate::steam_compat::SteamCompat;

pub struct DownloadResult {
    pub manifest_count: usize,
}

pub struct DownloadOrchestrator {
    hubcap_client: HubcapClient,
    steam_compat: SteamCompat,
}

impl DownloadOrchestrator {
    pub fn new(hubcap_client: HubcapClient, steam_compat: SteamCompat) -> Self {
        Self {
            hubcap_client,
            steam_compat,
        }
    }

    /// Downloads the Hubcap manifest ZIP once, installs the Lua into stplug-in,
    /// and preloads any bundled `.manifest` files into Steam/depotcache.
    pub async fn execute_hubcap_download(&self, app_id: u32) -> Result<DownloadResult, String> {
        let package = self.hubcap_client.download_lua_package(app_id).await?;

        self.steam_compat.install_lua_config(app_id, &package.lua_content)?;
        let manifest_count = self.steam_compat.install_manifest_files(&package.manifest_files)?;

        Ok(DownloadResult { manifest_count })
    }
}
