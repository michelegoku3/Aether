use serde::Serialize;
use tauri_plugin_dialog::{DialogExt, FilePath};
use crate::core::custom_css;
use crate::core::settings::SettingsManager;

/// Resolves the file selected by a native dialog into a local `PathBuf`.
/// Returns `Err("No file selected")` when the dialog was cancelled so the
/// frontend treats cancellation as a no-op, not an error.
fn picked_path_or_error(picked: Option<FilePath>) -> Result<std::path::PathBuf, String> {
    match picked {
        Some(FilePath::Path(path)) => Ok(path),
        Some(FilePath::Url(_)) => Err("Selected item is not a local file".to_string()),
        None => Err("No file selected".to_string()),
    }
}

/// Snapshot of the appearance assets available to the Settings UI:
/// whether a theme/wallpaper exists and which file is currently applied.
/// The frontend uses this to disable toggles when nothing is available.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceAssets {
    pub theme_exists: bool,
    pub theme_name: Option<String>,
    pub wallpaper_exists: bool,
    pub wallpaper_name: Option<String>,
    pub themes_dir: String,
    pub wallpapers_dir: String,
}

/// Returns the raw CSS text of the active theme
/// (first `.css` in `AetherData/config/themes/`, or the explicit selection).
#[tauri::command]
pub fn get_custom_css(app: tauri::AppHandle) -> Result<String, String> {
    let selected = SettingsManager::new(&app).load().theme_selected_file;
    custom_css::read_theme_css(&selected)
}

/// Absolute path of the active theme file (empty string when none exists).
#[tauri::command]
pub fn get_custom_css_path(app: tauri::AppHandle) -> Result<String, String> {
    let selected = SettingsManager::new(&app).load().theme_selected_file;
    Ok(custom_css::theme_path(&selected)?
        .map(|path| path.display().to_string())
        .unwrap_or_default())
}

/// Ensures `AetherData/config/themes/` and `AetherData/config/wallpapers/`
/// exist, seeding the embedded defaults (Cyberpunk + Frieren) on first run.
/// Returns the themes directory path. The legacy `custom.css` template is no
/// longer created: themes live exclusively in the `themes` folder.
#[tauri::command]
pub fn ensure_custom_css() -> Result<String, String> {
    custom_css::ensure_default_assets()?;
    Ok(custom_css::themes_dir().display().to_string())
}

/// Opens the folder containing the active theme in the system file explorer.
#[tauri::command]
pub fn open_custom_css_folder(_app: tauri::AppHandle) -> Result<(), String> {
    let folder = custom_css::themes_dir();
    open_folder_in_explorer(&folder)
}

/// Opens `AetherData/config/wallpapers/` in the system file explorer.
#[tauri::command]
pub fn open_wallpapers_folder() -> Result<(), String> {
    open_folder_in_explorer(&custom_css::wallpapers_dir())
}

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

/// Returns the absolute path of the active personal wallpaper file.
/// Empty string means no wallpaper file is configured.
#[tauri::command]
pub fn get_personal_wallpaper_path(app: tauri::AppHandle) -> Result<String, String> {
    let selected = SettingsManager::new(&app).load().wallpaper_selected_file;
    Ok(custom_css::personal_wallpaper_path(&selected)?
        .map(|path| path.display().to_string())
        .unwrap_or_default())
}

/// Returns the active personal wallpaper as a browser-ready data URI.
/// Empty string means no wallpaper file was found.
#[tauri::command]
pub fn get_personal_wallpaper_data_uri(app: tauri::AppHandle) -> Result<String, String> {
    let selected = SettingsManager::new(&app).load().wallpaper_selected_file;
    Ok(custom_css::read_personal_wallpaper_data_uri(&selected)?.unwrap_or_default())
}

/// Reports which appearance assets exist (theme + wallpaper) and their names,
/// plus the folders the file pickers will open. Used by the Settings UI to
/// enable/disable the toggles and to show "choose file" buttons.
#[tauri::command]
pub fn get_appearance_assets(app: tauri::AppHandle) -> Result<AppearanceAssets, String> {
    let settings = SettingsManager::new(&app).load();
    let theme_name = custom_css::active_theme_name(&settings.theme_selected_file);
    let wallpaper_name = custom_css::active_wallpaper_name(&settings.wallpaper_selected_file);
    Ok(AppearanceAssets {
        theme_exists: theme_name.is_some(),
        theme_name,
        wallpaper_exists: wallpaper_name.is_some(),
        wallpaper_name,
        themes_dir: custom_css::themes_dir().display().to_string(),
        wallpapers_dir: custom_css::wallpapers_dir().display().to_string(),
    })
}

/// Native file picker for themes. Opens directly inside
/// `AetherData/config/themes/` (with a `.css` filter); the user can navigate
/// elsewhere to import a new theme. The picked file is copied into the folder
/// (when not already there) and its file name is returned so the frontend can
/// persist it as `theme_selected_file`.
#[tauri::command]
pub fn pick_theme_file(app: tauri::AppHandle) -> Result<String, String> {
    let folder = custom_css::themes_dir();
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Failed to create themes folder: {}", e))?;

    let picked = app
        .dialog()
        .file()
        .set_title("Choose a theme (CSS)")
        .add_filter("CSS files", &["css"])
        .set_directory(folder.clone())
        .blocking_pick_file();

    let path = picked_path_or_error(picked)?;

    let res = custom_css::import_selected_file(&folder, &path);
    if let Ok(name) = &res {
        crate::desk_log_info!("appearance", "Selected theme file: '{}'", name);
    }
    res
}

/// Native file picker for wallpapers. Opens directly inside
/// `AetherData/config/wallpapers/` (with an image filter); the user can
/// navigate elsewhere to import a new wallpaper. The picked file is copied
/// into the folder (when not already there) and its file name is returned so
/// the frontend can persist it as `wallpaper_selected_file`.
#[tauri::command]
pub fn pick_wallpaper_file(app: tauri::AppHandle) -> Result<String, String> {
    let folder = custom_css::wallpapers_dir();
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Failed to create wallpapers folder: {}", e))?;

    let picked = app
        .dialog()
        .file()
        .set_title("Choose a wallpaper (image)")
        .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif"])
        .set_directory(folder.clone())
        .blocking_pick_file();

    let path = picked_path_or_error(picked)?;

    let res = custom_css::import_selected_file(&folder, &path);
    if let Ok(name) = &res {
        crate::desk_log_info!("appearance", "Selected wallpaper file: '{}'", name);
    }
    res
}
