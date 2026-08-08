use std::fs;
use std::path::PathBuf;
use crate::core::paths::LocalAppPaths;

/// Single source of truth for the custom CSS location.
/// Stored next to `settings.json` so the user finds it easily
/// and it benefits from the same `AetherData` Defender exclusion.
pub fn custom_css_path() -> PathBuf {
    LocalAppPaths::config_dir().join("custom.css")
}

/// Default template written when the file does not exist.
/// Commented-out so toggling ON with an untouched file does nothing,
/// but the user sees an example and where to write.
const DEFAULT_TEMPLATE: &str = r#"/* AetherDesk Custom CSS
   This file is loaded only when "Enable Custom CSS" is ON in Settings.
   Write your overrides after this comment. They are injected as a
   <style id="aether-custom-css"> tag after the default theme, so they
   win by cascade order.

   Examples:
   :root { --bg-app: #0a0a0f; --color-cyan: #ff00ff; }
   .sidebar { border-right: 2px solid var(--color-cyan); }
   .store-game-card { border-radius: 16px; }
*/

"#;

/// Ensure the parent directory and the file exist.
/// If the file is missing, creates it with `DEFAULT_TEMPLATE` and returns its path.
/// If it already exists, returns the path without touching it.
/// Fail-open: directory creation errors are propagated as `Err`, but callers
/// (Tauri commands) map them to user-visible messages without crashing the app.
pub fn ensure_custom_css() -> Result<PathBuf, String> {
    let path = custom_css_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create custom.css directory: {}", e))?;
    }
    if !path.exists() {
        fs::write(&path, DEFAULT_TEMPLATE)
            .map_err(|e| format!("Failed to create custom.css: {}", e))?;
    }
    Ok(path)
}

/// Read the custom CSS file.
/// - If the file does not exist → `Ok("")` (not an error, toggle can still be ON
///   but nothing is injected — the frontend shows "empty" and offers to create).
/// - If the file is unreadable → `Err` with a human message (shown in console, not as panic).
pub fn read_custom_css() -> Result<String, String> {
    let path = custom_css_path();
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(&path).map_err(|e| format!("Failed to read custom.css: {}", e))
}

/// Personal wallpaper file location discovery.
///
/// The user can place any browser-supported image named `wallpaper.<ext>` next
/// to `custom.css` in `AetherData/config/` (for example `wallpaper.png`,
/// `wallpaper.jpg`, `wallpaper.webp`, `wallpaper.gif`). We deliberately do not
/// hardcode a single extension: the browser decides whether the asset can be
/// rendered.
pub fn personal_wallpaper_path() -> Result<Option<PathBuf>, String> {
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

/// Read the configured personal wallpaper as a data URI. Returning a data URI
/// instead of a filesystem URL avoids Tauri asset-protocol scope issues and
/// works for arbitrary install/config paths.
pub fn read_personal_wallpaper_data_uri() -> Result<Option<String>, String> {
    let Some(path) = personal_wallpaper_path()? else {
        return Ok(None);
    };
    let bytes = fs::read(&path)
        .map_err(|e| format!("Failed to read wallpaper {}: {}", path.display(), e))?;
    let mime = wallpaper_mime(&path);
    Ok(Some(format!("data:{};base64,{}", mime, base64_encode(&bytes))))
}

fn wallpaper_mime(path: &std::path::Path) -> &'static str {
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
