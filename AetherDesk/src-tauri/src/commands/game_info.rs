use crate::game_info::model::GameInfo;
use crate::game_info::service::GameInfoService;

#[tauri::command]
pub async fn get_game_info(app: tauri::AppHandle, app_id: u32) -> Result<GameInfo, String> {
    crate::desk_log_debug!("game_info", "Fetching game info for AppID {}", app_id);
    let res = GameInfoService::new(app).get_game_info(app_id).await;
    match &res {
        Ok(info) => crate::desk_log_info!(
            "game_info",
            "Successfully fetched game info for AppID {}: '{}'",
            app_id,
            info.name.as_deref().unwrap_or("Unknown")
        ),
        Err(e) => crate::desk_log_warn!(
            "game_info",
            "Failed to fetch game info for AppID {}: {}",
            app_id,
            e
        ),
    }
    res
}

pub(crate) fn build_game_info_link_url(app_id: u32, site: &str) -> Result<String, String> {
    if app_id == 0 {
        return Err("A valid Steam App ID is required".to_string());
    }
    match site {
        "steam" => Ok(format!("https://store.steampowered.com/app/{app_id}/")),
        "steamdb" => Ok(format!("https://steamdb.info/app/{app_id}/")),
        "pcgw" => Ok(format!(
            "https://www.pcgamingwiki.com/api/appid.php?appid={app_id}"
        )),
        _ => Err(format!("Unsupported game info link: {site}")),
    }
}

#[tauri::command]
pub fn open_game_info_link(app_id: u32, site: String) -> Result<(), String> {
    let url = build_game_info_link_url(app_id, site.trim())?;
    crate::desk_log_info!(
        "game_info",
        "Opening external link site='{}' for AppID {}",
        site,
        app_id
    );
    crate::util::browser::open_external_url(&url)
}

#[cfg(test)]
mod tests {
    use super::build_game_info_link_url;

    #[test]
    fn builds_allowlisted_urls_and_rejects_invalid_inputs() {
        assert_eq!(
            build_game_info_link_url(730, "steam").unwrap(),
            "https://store.steampowered.com/app/730/"
        );
        assert_eq!(
            build_game_info_link_url(730, "steamdb").unwrap(),
            "https://steamdb.info/app/730/"
        );
        assert_eq!(
            build_game_info_link_url(730, "pcgw").unwrap(),
            "https://www.pcgamingwiki.com/api/appid.php?appid=730"
        );
        assert!(build_game_info_link_url(0, "steam").is_err());
        assert!(build_game_info_link_url(730, "evil").is_err());
    }
}
