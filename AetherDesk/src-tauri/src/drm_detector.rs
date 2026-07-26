use serde::Deserialize;
use std::collections::HashMap;

const STEAM_APPDETAILS_URL: &str = "https://store.steampowered.com/api/appdetails";
const DENUVO_MARKER: &str = "denuvo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrmStatus {
    Clean,
    Denuvo,
    Unknown,
}

impl DrmStatus {
    pub fn has_denuvo(self) -> bool {
        matches!(self, Self::Denuvo)
    }
}

#[derive(Debug, Deserialize)]
struct AppDetailsEnvelope {
    success: bool,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

/// Small, focused client for Steam Store DRM metadata.
///
/// This intentionally mirrors the safe part of SFF's approach: Steam already exposes
/// DRM notes in the public appdetails endpoint, and Denuvo is identified by checking
/// whether `data.drm_notice` contains "denuvo".
///
/// Keeping this in its own module avoids coupling StoreService to Steam's appdetails
/// response shape and makes it easy to add more DRM tags later.
pub struct DrmDetector {
    client: reqwest::Client,
}

impl DrmDetector {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    pub async fn detect(&self, app_id: u32) -> DrmStatus {
        match self.fetch_drm_notice(app_id).await {
            Ok(Some(notice)) if notice.to_lowercase().contains(DENUVO_MARKER) => DrmStatus::Denuvo,
            Ok(Some(_)) | Ok(None) => DrmStatus::Clean,
            Err(_) => DrmStatus::Unknown,
        }
    }

    async fn fetch_drm_notice(&self, app_id: u32) -> Result<Option<String>, String> {
        let response = self
            .client
            .get(STEAM_APPDETAILS_URL)
            .query(&[("appids", app_id.to_string()), ("l", "english".to_string())])
            .send()
            .await
            .map_err(|e| format!("Steam DRM check network error for {}: {}", app_id, e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Steam DRM check returned HTTP error for {}: {}",
                app_id,
                response.status()
            ));
        }

        let data = response
            .json::<HashMap<String, AppDetailsEnvelope>>()
            .await
            .map_err(|e| format!("Failed to parse Steam DRM response for {}: {}", app_id, e))?;

        let envelope = data
            .get(&app_id.to_string())
            .ok_or_else(|| format!("Steam DRM response missing app id {}", app_id))?;

        if !envelope.success {
            return Ok(None);
        }

        Ok(envelope
            .data
            .as_ref()
            .and_then(|data| data.get("drm_notice"))
            .and_then(|notice| notice.as_str())
            .map(|notice| notice.to_string()))
    }
}
