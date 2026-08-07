use crate::core::custom_css;

/// Returns the raw CSS text of `AetherData/config/custom.css`.
/// - If the file does not exist → `Ok("")` (frontend shows “empty” hint, not an error popup).
/// - If the file cannot be read → `Err` with a human-readable message.
/// This command is free and never touches the network; it is the single
/// place where the frontend obtains custom CSS content (SRP: file I/O isolated here).
#[tauri::command]
pub fn get_custom_css() -> Result<String, String> {
    custom_css::read_custom_css()
}

/// Returns the absolute path of the custom CSS file as a string.
/// Useful for the Settings UI to show the user where to edit.
#[tauri::command]
pub fn get_custom_css_path() -> Result<String, String> {
    Ok(custom_css::custom_css_path().display().to_string())
}

/// Ensures `AetherData/config/custom.css` exists (creates it with a commented
/// template if missing) and returns its absolute path.
/// The frontend calls this after the user toggles ON, so the file is always
/// present for manual editing.
#[tauri::command]
pub fn ensure_custom_css() -> Result<String, String> {
    let path = custom_css::ensure_custom_css()?;
    Ok(path.display().to_string())
}

/// Opens the folder containing `custom.css` in the system file explorer.
/// - On Windows: `explorer <folder>`
/// - On macOS: `open <folder>`
/// - On Linux: `xdg-open <folder>`
/// Fail-open: if the folder cannot be opened, returns an `Err` that the
/// frontend shows as a console warning, not a panic.
#[tauri::command]
pub fn open_custom_css_folder() -> Result<(), String> {
    let path = custom_css::ensure_custom_css()?;
    let folder = path
        .parent()
        .ok_or_else(|| "Custom CSS path has no parent".to_string())?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}
