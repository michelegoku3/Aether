use crate::hubcap_client::HubcapClient;
use crate::steam_compat::SteamCompat;

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

    /// Executes the first professional pipeline option: Download LUA directly from Hubcap
    /// and deploy it cleanly to Steam, preparing the environment.
    pub async fn execute_hubcap_download(&self, app_id: u32) -> Result<String, String> {
        // Step 1: Download LUA decryption keys from Hubcap
        let lua_content = self.hubcap_client.download_lua_config(app_id).await?;

        // Step 2: Install LUA to the Steam plugin directory cleanly
        self.steam_compat.install_lua_config(app_id, &lua_content)?;

        // Return confirmation
        Ok(format!(
            "Successfully completed Hubcap download pipeline for App ID {}. Decryption LUA keys installed safely.",
            app_id
        ))
    }
}
