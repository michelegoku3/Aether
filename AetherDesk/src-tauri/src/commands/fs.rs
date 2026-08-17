//! Comandi filesystem generici e riusabili (apertura nel file manager).
//!
//! Separati dai comandi di dominio (es. `custom_css`) per alta coesione:
//! ogni modulo ha una sola responsabilità e i comandi di dominio non devono
//! esporre infrastruttura fs generica.

/// Generic, reusable command: opens any folder in the system file manager.
/// Used by the Online panel (game exe/DLL folders) and by the Settings view
/// (themes, wallpapers, icons folders).
#[tauri::command]
pub fn reveal_in_file_manager(path: String) -> Result<(), String> {
    crate::desk_log_info!("fs", "Opening folder in OS file manager: {path}");
    open_folder_in_explorer(std::path::Path::new(&path))
}

/// Opens `folder` in the platform file manager (explorer / open / xdg-open),
/// creating it first if it does not exist yet.
fn open_folder_in_explorer(folder: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("Failed to create folder {}: {}", folder.display(), e))?;

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
