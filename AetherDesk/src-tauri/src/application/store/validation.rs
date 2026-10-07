//! Validation performed before provider calls or Steam-side writes.

pub(super) fn validate_steam_download_path(steam_path: &str) -> Result<(), String> {
    crate::util::validation::validate_steam_path(steam_path)
}

pub(super) fn validate_authenticated_download(
    api_key: &str,
    steam_path: &str,
    api_action: &str,
) -> Result<(), String> {
    if api_key.trim().is_empty() {
        let error = format!("API Key is required to {api_action}");
        crate::desk_log_error!("store", "Download validation failed: {}", error);
        return Err(error);
    }
    if let Err(error) = crate::util::validation::validate_steam_path(steam_path) {
        crate::desk_log_error!("store", "Download validation failed: {}", error);
        return Err(error);
    }
    Ok(())
}
