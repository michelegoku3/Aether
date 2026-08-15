use std::fs;
use std::path::{Path, PathBuf};
use crate::core::paths::LocalAppPaths;

/// Custom themes live in `AetherData/config/themes/`.
/// The *first* `.css` file found (sorted by file name) is the active theme.
/// A file can be explicitly selected via settings (`theme_selected_file`);
/// when it is missing or empty, the first-detected file wins.
pub fn themes_dir() -> PathBuf {
    LocalAppPaths::config_dir().join("themes")
}

/// Personal wallpapers live in `AetherData/config/wallpapers/`.
/// The *first* image file found (sorted by file name) is the active wallpaper.
/// A file can be explicitly selected via settings (`wallpaper_selected_file`);
/// when it is missing or empty, the first-detected file wins.
pub fn wallpapers_dir() -> PathBuf {
    LocalAppPaths::config_dir().join("wallpapers")
}

/// Custom window icons live in `AetherData/config/icons/`.
pub fn icons_dir() -> PathBuf {
    LocalAppPaths::config_dir().join("icons")
}

/// Default Goldmine theme shipped with AetherDesk (embedded in the binary so
/// no bundle/resource path resolution is needed in dev or in production).
/// NOTE: `include_str!` paths are relative to THIS source file, so two `..`
/// levels are required to reach `src-tauri/assets/...` from `src/core/`.
const DEFAULT_CYBERPUNK_THEME: &str = include_str!("../../assets/defaults/themes/cyberpunk.css");

/// Goldmine theme (gold/ivory/dark — originally shipped as "frieren").
const DEFAULT_GOLDMINE_THEME: &str = include_str!("../../assets/defaults/themes/goldmine.css");

/// Frieren theme (Frieren • Fern • Stark palette).
const DEFAULT_FRIEREN_THEME: &str = include_str!("../../assets/defaults/themes/frieren.css");

/// Halo theme (UNSC / Master Chief / MJOLNIR palette).
const DEFAULT_HALO_THEME: &str = include_str!("../../assets/defaults/themes/halo.css");

/// Default wallpaper shipped with AetherDesk (4K Cyberpunk 2077 art, embedded).
const DEFAULT_CYBERPUNK_WALLPAPER: &[u8] = include_bytes!("../../assets/defaults/wallpapers/cyberpunk.jpg");

/// Default Frieren wallpaper shipped with AetherDesk.
const DEFAULT_FRIEREN_WALLPAPER: &[u8] = include_bytes!("../../assets/defaults/wallpapers/frieren.jpg");

/// Default Goldmine wallpaper shipped with AetherDesk.
const DEFAULT_GOLDMINE_WALLPAPER: &[u8] = include_bytes!("../../assets/defaults/wallpapers/goldmine.jpg");

/// Default Halo wallpaper shipped with AetherDesk.
const DEFAULT_HALO_WALLPAPER: &[u8] = include_bytes!("../../assets/defaults/wallpapers/halo.jpg");

/// File names used when writing the embedded defaults into the user folders.
const DEFAULT_THEME_FILES: &[(&str, &str)] = &[
    ("cyberpunk.css", DEFAULT_CYBERPUNK_THEME),
    ("goldmine.css", DEFAULT_GOLDMINE_THEME),
    ("frieren.css", DEFAULT_FRIEREN_THEME),
    ("halo.css", DEFAULT_HALO_THEME),
];
const DEFAULT_WALLPAPER_FILES: &[(&str, &[u8])] = &[
    ("cyberpunk.jpg", DEFAULT_CYBERPUNK_WALLPAPER),
    ("goldmine.jpg", DEFAULT_GOLDMINE_WALLPAPER),
    ("frieren.jpg", DEFAULT_FRIEREN_WALLPAPER),
    ("halo.jpg", DEFAULT_HALO_WALLPAPER),
];

/// Official AetherDesk window icon (bundled Tauri icon resource).
/// Used whenever custom icons are disabled and as the shell-shortcut default.
pub const DEFAULT_AETHER_ICON: &[u8] = include_bytes!("../../icons/icon.ico");

/// Optional meme / alternate icon shipped into `config/icons/` for the picker.
/// Named explicitly so it is never confused with the official app icon.
const DEFAULT_ICON_FILES: &[(&str, &[u8])] = &[(
    "aether_genshin.ico",
    include_bytes!("../../assets/defaults/icons/aether_genshin.ico"),
)];

/// Image extensions the wallpaper picker accepts (browser-renderable set).
const WALLPAPER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif", "ico"];

/// Raster / icon formats we can decode into a window icon.
pub const ICON_EXTENSIONS: &[&str] = &[
    "ico", "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff",
];

// ---------------------------------------------------------------------------
// First-run provisioning
// ---------------------------------------------------------------------------

/// Creates `config/themes/` and `config/wallpapers/` (if missing) and seeds
/// them with the embedded defaults (Cyberpunk + Frieren themes and wallpapers)
/// when the folders are empty.
/// Idempotent: existing files are never overwritten, so user files survive
/// upgrades. Failures are logged, never fatal.
pub fn ensure_default_assets() -> Result<(), String> {
    let themes_dir = themes_dir();
    let wallpapers_dir = wallpapers_dir();
    let icons_dir = icons_dir();

    fs::create_dir_all(&themes_dir)
        .map_err(|e| format!("Failed to create themes directory: {}", e))?;
    fs::create_dir_all(&wallpapers_dir)
        .map_err(|e| format!("Failed to create wallpapers directory: {}", e))?;
    fs::create_dir_all(&icons_dir)
        .map_err(|e| format!("Failed to create icons directory: {}", e))?;

    for (file_name, content) in DEFAULT_THEME_FILES {
        let file = themes_dir.join(file_name);
        if !file.exists() {
            if let Err(e) = fs::write(&file, content) {
                eprintln!("[AetherDesk] failed to write default theme {}: {}", file_name, e);
            }
        }
    }

    for (file_name, content) in DEFAULT_WALLPAPER_FILES {
        let file = wallpapers_dir.join(file_name);
        if !file.exists() {
            if let Err(e) = fs::write(&file, content) {
                eprintln!("[AetherDesk] failed to write default wallpaper {}: {}", file_name, e);
            }
        }
    }

    // Migrate legacy meme icon name from older builds.
    let legacy_meme = icons_dir.join("aether.ico");
    let renamed_meme = icons_dir.join("aether_genshin.ico");
    if legacy_meme.is_file() && !renamed_meme.exists() {
        if let Err(e) = fs::rename(&legacy_meme, &renamed_meme) {
            eprintln!(
                "[AetherDesk] failed to rename legacy icon {} → {}: {}",
                legacy_meme.display(),
                renamed_meme.display(),
                e
            );
        }
    }

    for (file_name, content) in DEFAULT_ICON_FILES {
        let file = icons_dir.join(file_name);
        if !file.exists() {
            if let Err(e) = fs::write(&file, content) {
                eprintln!("[AetherDesk] failed to write default icon {}: {}", file_name, e);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Theme helpers
// ---------------------------------------------------------------------------

/// Lists `.css` files inside `config/themes/`, sorted by file name so the
/// "first detected" default is deterministic.
fn theme_candidates() -> Result<Vec<PathBuf>, String> {
    list_files_with_extension(&themes_dir(), "css")
}

/// Active theme path:
/// 1. explicitly selected file (`theme_selected_file`) when it still exists;
/// 2. otherwise the first `.css` file in `config/themes/`;
/// 3. otherwise `None`.
pub fn theme_path(selected_file: &str) -> Result<Option<PathBuf>, String> {
    let selected = resolve_selected(&themes_dir(), selected_file)?;
    if let Some(path) = selected {
        return Ok(Some(path));
    }
    let candidates = theme_candidates()?;
    Ok(candidates.into_iter().next())
}

/// Read the active theme CSS content. Empty string when no theme exists.
pub fn read_theme_css(selected_file: &str) -> Result<String, String> {
    let Some(path) = theme_path(selected_file)? else {
        return Ok(String::new());
    };
    fs::read_to_string(&path).map_err(|e| format!("Failed to read theme {}: {}", path.display(), e))
}

// ---------------------------------------------------------------------------
// Personal wallpaper helpers
// ---------------------------------------------------------------------------

/// Active wallpaper path:
/// 1. explicitly selected file (`wallpaper_selected_file`) when it still exists;
/// 2. otherwise the first image file in `config/wallpapers/` (sorted by name);
/// 3. otherwise the legacy `config/wallpaper.<ext>` file when present;
/// 4. otherwise `None`.
pub fn personal_wallpaper_path(selected_file: &str) -> Result<Option<PathBuf>, String> {
    let selected = resolve_selected(&wallpapers_dir(), selected_file)?;
    if let Some(path) = selected {
        return Ok(Some(path));
    }

    let candidates = wallpaper_candidates()?;
    if let Some(first) = candidates.into_iter().next() {
        return Ok(Some(first));
    }

    // Legacy single-file location (`config/wallpaper.<ext>`) still honoured.
    legacy_wallpaper_path()
}

/// Lists image files inside `config/wallpapers/`, sorted by file name.
fn wallpaper_candidates() -> Result<Vec<PathBuf>, String> {
    list_files_with_extension(&wallpapers_dir(), "img")
}

/// `config/wallpaper.<ext>` — the pre-folder layout, kept for compatibility.
fn legacy_wallpaper_path() -> Result<Option<PathBuf>, String> {
    let config_dir = LocalAppPaths::config_dir();
    if !config_dir.is_dir() {
        return Ok(None);
    }
    let entries = fs::read_dir(&config_dir)
        .map_err(|e| format!("Failed to read config folder for wallpaper: {}", e))?;

    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.eq_ignore_ascii_case("wallpaper"))
                .unwrap_or(false)
        })
        .collect();

    candidates.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(candidates.into_iter().next())
}

/// Read the active wallpaper as a browser-ready data URI. Empty string when no
/// wallpaper exists. Data URIs avoid Tauri asset-protocol scope issues.
pub fn read_personal_wallpaper_data_uri(selected_file: &str) -> Result<Option<String>, String> {
    let Some(path) = personal_wallpaper_path(selected_file)? else {
        return Ok(None);
    };
    let bytes = fs::read(&path)
        .map_err(|e| format!("Failed to read wallpaper {}: {}", path.display(), e))?;
    let mime = wallpaper_mime(&path);
    Ok(Some(format!("data:{};base64,{}", mime, base64_encode(&bytes))))
}

/// Name of the file currently applied (selected or first detected), for UI
/// feedback. `None` when nothing is available.
pub fn active_wallpaper_name(selected_file: &str) -> Option<String> {
    personal_wallpaper_path(selected_file)
        .ok()
        .flatten()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
}

/// Name of the theme currently applied (selected or first detected), for UI
/// feedback. `None` when nothing is available.
pub fn active_theme_name(selected_file: &str) -> Option<String> {
    theme_path(selected_file)
        .ok()
        .flatten()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
}

pub fn icon_path(selected_file: &str) -> Result<Option<PathBuf>, String> {
    let selected = resolve_selected(&icons_dir(), selected_file)?;
    if let Some(path) = selected {
        return Ok(Some(path));
    }
    Ok(icon_candidates()?.into_iter().next())
}

fn icon_candidates() -> Result<Vec<PathBuf>, String> {
    list_files_with_extension(&icons_dir(), "icon")
}

pub fn active_icon_name(selected_file: &str) -> Option<String> {
    icon_path(selected_file)
        .ok()
        .flatten()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
}

fn icon_from_bytes(bytes: &[u8]) -> Result<tauri::image::Image<'static>, String> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|e| format!("Unsupported icon format: {}", e))?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err("Icon image is empty".to_string());
    }
    Ok(tauri::image::Image::new_owned(rgba.into_raw(), width, height))
}

pub fn load_window_icon(path: &Path) -> Result<tauri::image::Image<'static>, String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read icon {}: {}", path.display(), e))?;
    icon_from_bytes(&bytes)
}

pub fn default_window_icon() -> Result<tauri::image::Image<'static>, String> {
    icon_from_bytes(DEFAULT_AETHER_ICON)
}

pub fn apply_window_icon(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    let settings = crate::core::settings::SettingsManager::new(app).load();
    let image = if settings.custom_icon_enabled {
        match icon_path(&settings.icon_selected_file)? {
            Some(path) => load_window_icon(&path)?,
            None => default_window_icon()?,
        }
    } else {
        // Custom icons OFF → always the official bundled icon, never the
        // first file sitting in config/icons/ (that folder is only a picker source).
        default_window_icon()?
    };

    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    window
        .set_icon(image)
        .map_err(|e| format!("Failed to set window icon: {}", e))?;

    // Shortcuts / shell cache: custom source when enabled, official default otherwise.
    let source = if settings.custom_icon_enabled {
        icon_path(&settings.icon_selected_file).ok().flatten()
    } else {
        None
    };
    match crate::core::shell_shortcuts::materialize_shell_ico(source.as_deref()) {
        Ok(shell_ico) => crate::core::shell_shortcuts::sync_windows_shortcuts(&shell_ico),
        Err(e) => eprintln!("[AetherDesk] shell icon materialize failed: {e}"),
    }
    Ok(())
}

/// Restore title-bar + Start Menu / Desktop shortcuts to the official AetherDesk
/// icon. Safe to call without an `AppHandle` (uninstall helper, CLI paths).
pub fn restore_default_window_icon(app: Option<&tauri::AppHandle>) -> Result<(), String> {
    if let Some(app) = app {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            let image = default_window_icon()?;
            window
                .set_icon(image)
                .map_err(|e| format!("Failed to restore window icon: {}", e))?;
        }
    }

    let shell_ico = crate::core::shell_shortcuts::materialize_shell_ico(None)?;
    crate::core::shell_shortcuts::sync_windows_shortcuts(&shell_ico);
    Ok(())
}

// ---------------------------------------------------------------------------
// Selection / import helpers (used by the native file picker commands)
// ---------------------------------------------------------------------------

/// Copies the user-picked file into the target folder (when it is not already
/// inside it) and returns the file name stored in settings as the selection.
/// The picker is seeded with the folder as start directory, but the user can
/// navigate elsewhere to import a brand-new wallpaper/theme.
pub fn import_selected_file(folder: &Path, picked_path: &Path) -> Result<String, String> {
    let picked_path = picked_path.to_path_buf();
    let file_name = picked_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| "Picked file has no valid file name".to_string())?;

    let is_already_inside = picked_path
        .parent()
        .map(|parent| parent == folder)
        .unwrap_or(false);

    if !is_already_inside {
        fs::create_dir_all(folder)
            .map_err(|e| format!("Failed to create folder {}: {}", folder.display(), e))?;
        let destination = folder.join(file_name);
        if destination.exists() {
            return Err(format!(
                "A file named '{}' already exists in {}. Rename it and try again.",
                file_name,
                folder.display()
            ));
        }
        fs::copy(&picked_path, &destination)
            .map_err(|e| format!("Failed to copy {} into {}: {}", picked_path.display(), folder.display(), e))?;
    }

    Ok(file_name.to_string())
}

/// Resolves a stored selection: only when the folder exists, the file name is
/// non-empty and the file still exists. Otherwise `None` (fall back to first).
fn resolve_selected(folder: &Path, selected_file: &str) -> Result<Option<PathBuf>, String> {
    let selected_file = selected_file.trim();
    if selected_file.is_empty() {
        return Ok(None);
    }
    let candidate = folder.join(selected_file);
    if candidate.is_file() {
        Ok(Some(candidate))
    } else {
        Ok(None)
    }
}

/// Lists files inside `dir` filtered by `kind` ("css" or "img"), sorted by
/// file name. Missing folder → empty vec (not an error).
fn list_files_with_extension(dir: &Path, kind: &str) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read folder {}: {}", dir.display(), e))?;
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| match kind {
            "css" => path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("css"))
                .unwrap_or(false),
            "icon" => path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    ICON_EXTENSIONS
                        .iter()
                        .any(|allowed| ext.eq_ignore_ascii_case(allowed))
                })
                .unwrap_or(false),
            _ => path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    WALLPAPER_EXTENSIONS
                        .iter()
                        .any(|allowed| ext.eq_ignore_ascii_case(allowed))
                })
                .unwrap_or(false),
        })
        .collect();
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

fn wallpaper_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((bytes.len() + 2) / 3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);

        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }

        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }

    out
}
