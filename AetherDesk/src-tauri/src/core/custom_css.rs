use crate::core::paths::LocalAppPaths;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Theme-authoring kit: a read-only reference copy of the stylesheet the app
/// itself uses, so a theme author (or an AI) starts from the full class
/// inventory instead of guessing. The stylesheet is rewritten on every start,
/// so it always matches the running build.
pub fn theme_kit_dir() -> PathBuf {
    LocalAppPaths::config_dir().join("theme-kit")
}

/// The app's own stylesheet, copied into `OUT_DIR` by `build.rs` and embedded
/// here. The theme kit writes it out as `default.css`, so the reference an
/// author receives is byte-identical to the CSS the UI loads.
pub const DEFAULT_STYLESHEET: &str = include_str!(concat!(env!("OUT_DIR"), "/default.css"));

/// Default Cyberpunk theme shipped with AetherDesk (embedded in the binary so
/// no bundle/resource path resolution is needed in dev or in production).
/// NOTE: `include_str!` paths are relative to THIS source file, so two `..`
/// levels are required to reach `src-tauri/assets/...` from `src/core/`.
const DEFAULT_CYBERPUNK_THEME: &str = include_str!("../../assets/defaults/themes/cyberpunk.css");

/// Goldmine theme (gold/ivory/dark).
const DEFAULT_GOLDMINE_THEME: &str = include_str!("../../assets/defaults/themes/goldmine.css");

/// Frieren theme (Frieren • Fern • Stark palette).
const DEFAULT_FRIEREN_THEME: &str = include_str!("../../assets/defaults/themes/frieren.css");

/// Halo theme (UNSC / Master Chief / MJOLNIR palette).
const DEFAULT_HALO_THEME: &str = include_str!("../../assets/defaults/themes/halo.css");

/// Wonderland theme (TV-world: pixel-art type, CRT scanlines, paper modals).
const DEFAULT_WONDERLAND_THEME: &str = include_str!("../../assets/defaults/themes/wonderland.css");

/// Default wallpaper shipped with AetherDesk (4K Cyberpunk 2077 art, embedded).
const DEFAULT_CYBERPUNK_WALLPAPER: &[u8] =
    include_bytes!("../../assets/defaults/wallpapers/cyberpunk.jpg");

/// Default Frieren wallpaper shipped with AetherDesk.
const DEFAULT_FRIEREN_WALLPAPER: &[u8] =
    include_bytes!("../../assets/defaults/wallpapers/frieren.jpg");

/// Default Goldmine wallpaper shipped with AetherDesk.
const DEFAULT_GOLDMINE_WALLPAPER: &[u8] =
    include_bytes!("../../assets/defaults/wallpapers/goldmine.jpg");

/// Default Halo wallpaper shipped with AetherDesk.
const DEFAULT_HALO_WALLPAPER: &[u8] = include_bytes!("../../assets/defaults/wallpapers/halo.jpg");

/// File names used when writing the embedded defaults into the user folders.
const DEFAULT_THEME_FILES: &[(&str, &str)] = &[
    ("cyberpunk.css", DEFAULT_CYBERPUNK_THEME),
    ("goldmine.css", DEFAULT_GOLDMINE_THEME),
    ("frieren.css", DEFAULT_FRIEREN_THEME),
    ("halo.css", DEFAULT_HALO_THEME),
    ("wonderland.css", DEFAULT_WONDERLAND_THEME),
];
const DEFAULT_WALLPAPER_FILES: &[(&str, &[u8])] = &[
    ("cyberpunk.jpg", DEFAULT_CYBERPUNK_WALLPAPER),
    ("goldmine.jpg", DEFAULT_GOLDMINE_WALLPAPER),
    ("frieren.jpg", DEFAULT_FRIEREN_WALLPAPER),
    ("halo.jpg", DEFAULT_HALO_WALLPAPER),
];

/// Bookkeeping for the default themes we wrote: `themes/<file>` -> SHA-256 of
/// the content we last shipped. A file whose content still hashes to its entry
/// is untouched, so a later release may refresh it; a file with any other
/// content belongs to the user and is never overwritten.
const DEFAULTS_MANIFEST_FILE: &str = ".aether-defaults.json";
const DEFAULTS_MANIFEST_SCHEMA: u32 = 1;

/// Default themes as shipped through desk-1.4.3 (byte-identical to 1.3.x).
/// Installs that carry one of these copies predate the manifest, so the hash is
/// the only evidence that the file is still a seed: a match is refreshed with
/// the current default, anything else belongs to the user.
const LEGACY_DEFAULT_THEME_HASHES: &[(&str, &str)] = &[
    (
        "cyberpunk.css",
        "0d19336ac5cc298811d2c8d5ac05b1ef571c78ed66c1d470748ca646d04ca598",
    ),
    (
        "frieren.css",
        "b25fb43051291a88d0785e2610ced6ab018377fcb81c9058924e2a221bda585f",
    ),
    (
        "goldmine.css",
        "6cc9a4d4b2bd8771278d730717511fae58af99ede53b5463b240dbb3d38f486b",
    ),
    (
        "halo.css",
        "6a43d8c4567d253c5be2851466d12bcf5fd1acc99e59bd5358c45fca3f77b219",
    ),
    (
        "wonderland.css",
        "3db634dca3e39d3d164edbf17895a2f4178174e168c4dbd1554805363101ba58",
    ),
];

#[derive(Debug, Default, Serialize, Deserialize)]
struct DefaultsManifest {
    #[serde(default)]
    schema: u32,
    /// Only themes are managed; wallpapers and icons are seeded once and then
    /// belong to the user.
    #[serde(default)]
    themes: BTreeMap<String, String>,
}

/// What happened to one default theme during provisioning.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DefaultThemeOutcome {
    Created,
    Refreshed,
    Unchanged,
    /// Existing content that is neither a tracked seed nor a legacy one.
    Kept,
}

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

/// Creates `config/themes/`, `config/wallpapers/`, `config/icons/` and
/// `config/theme-kit/` (if missing) and seeds the theme, wallpaper and icon
/// folders with the embedded defaults (Cyberpunk, Goldmine, Frieren, Halo and
/// Wonderland themes; Cyberpunk, Goldmine, Frieren and Halo wallpapers; plus
/// the alternate icon). The theme kit's `default.css` is rewritten every time
/// so it always matches the running stylesheet.
///
/// Default themes are **managed**: the manifest records the hash of the copy we
/// wrote, so a later release refreshes an untouched file while an edited one is
/// left alone. Wallpapers and icons are seeded only when missing, so user files
/// always survive upgrades. Failures are logged, never fatal.
pub fn ensure_default_assets() -> Result<(), String> {
    let themes_dir = themes_dir();
    let wallpapers_dir = wallpapers_dir();
    let icons_dir = icons_dir();
    let theme_kit_dir = theme_kit_dir();

    fs::create_dir_all(&themes_dir)
        .map_err(|e| format!("Failed to create themes directory: {}", e))?;
    fs::create_dir_all(&wallpapers_dir)
        .map_err(|e| format!("Failed to create wallpapers directory: {}", e))?;
    fs::create_dir_all(&icons_dir)
        .map_err(|e| format!("Failed to create icons directory: {}", e))?;
    fs::create_dir_all(&theme_kit_dir)
        .map_err(|e| format!("Failed to create theme kit directory: {}", e))?;

    // The kit is a reference, not a user file: refreshing it on every start is
    // what keeps `default.css` identical to the stylesheet in use.
    let kit_stylesheet = theme_kit_dir.join("default.css");
    if let Err(e) = fs::write(&kit_stylesheet, DEFAULT_STYLESHEET) {
        eprintln!(
            "[AetherDesk] failed to refresh theme kit {}: {}",
            kit_stylesheet.display(),
            e
        );
    }

    // Default themes follow the app across releases when the user never edited
    // them. The manifest is the evidence of "this content is still ours".
    let mut manifest = load_defaults_manifest();
    manifest.schema = DEFAULTS_MANIFEST_SCHEMA;
    for (file_name, content) in DEFAULT_THEME_FILES {
        if sync_default_theme(
            file_name,
            content,
            &mut manifest.themes,
            LEGACY_DEFAULT_THEME_HASHES,
        ) == DefaultThemeOutcome::Refreshed
        {
            eprintln!("[AetherDesk] refreshed untouched default theme {file_name}");
        }
    }
    // Entries for themes this build no longer ships would keep stale hashes
    // around; drop them so the manifest describes exactly the current set.
    manifest
        .themes
        .retain(|name, _| DEFAULT_THEME_FILES.iter().any(|(file, _)| file == name));
    save_defaults_manifest(&manifest);

    for (file_name, content) in DEFAULT_WALLPAPER_FILES {
        let file = wallpapers_dir.join(file_name);
        if !file.exists() {
            if let Err(e) = fs::write(&file, content) {
                eprintln!(
                    "[AetherDesk] failed to write default wallpaper {}: {}",
                    file_name, e
                );
            }
        }
    }

    for (file_name, content) in DEFAULT_ICON_FILES {
        let file = icons_dir.join(file_name);
        if !file.exists() {
            if let Err(e) = fs::write(&file, content) {
                eprintln!(
                    "[AetherDesk] failed to write default icon {}: {}",
                    file_name, e
                );
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Managed default themes
// ---------------------------------------------------------------------------

fn defaults_manifest_path() -> PathBuf {
    themes_dir().join(DEFAULTS_MANIFEST_FILE)
}

fn load_defaults_manifest() -> DefaultsManifest {
    let path = defaults_manifest_path();
    let Ok(content) = fs::read_to_string(&path) else {
        return DefaultsManifest::default();
    };
    match serde_json::from_str(&content) {
        Ok(manifest) => manifest,
        Err(error) => {
            // A corrupted manifest must not block startup: without tracked
            // hashes every default is treated as a possible user file.
            eprintln!(
                "[AetherDesk] ignoring unreadable defaults manifest {}: {}",
                path.display(),
                error
            );
            DefaultsManifest::default()
        }
    }
}

fn save_defaults_manifest(manifest: &DefaultsManifest) {
    let path = defaults_manifest_path();
    let Ok(serialized) = serde_json::to_string_pretty(manifest) else {
        return;
    };
    if fs::read_to_string(&path).is_ok_and(|current| current == serialized) {
        return;
    }
    if let Err(error) = fs::write(&path, serialized) {
        eprintln!(
            "[AetherDesk] failed to write defaults manifest {}: {}",
            path.display(),
            error
        );
    }
}

/// SHA-256 of a theme file with CRLF folded to LF, so the embedded default and
/// the copy an older build wrote compare equal whatever the checkout used.
pub(crate) fn content_hash(bytes: &[u8]) -> String {
    let normalized: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|byte| *byte != b'\r')
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(&normalized);
    format!("{:x}", hasher.finalize())
}

/// Writes, refreshes or preserves one default theme:
/// - missing file: write it and record our hash;
/// - content equal to the embedded default: nothing to do;
/// - content equal to what we recorded or to a legacy 1.4.3 hash: an untouched
///   seed, so replace it with the current default;
/// - anything else: the user edited or replaced it, keep the file forever.
pub(crate) fn sync_default_theme(
    file_name: &str,
    content: &str,
    tracked: &mut BTreeMap<String, String>,
    legacy: &[(&str, &str)],
) -> DefaultThemeOutcome {
    let path = themes_dir().join(file_name);
    let embedded_hash = content_hash(content.as_bytes());

    if !path.exists() {
        return match fs::write(&path, content) {
            Ok(()) => {
                tracked.insert(file_name.to_string(), embedded_hash);
                DefaultThemeOutcome::Created
            }
            Err(error) => {
                eprintln!(
                    "[AetherDesk] failed to write default theme {}: {}",
                    path.display(),
                    error
                );
                DefaultThemeOutcome::Kept
            }
        };
    }

    let current_hash = match fs::read(&path) {
        Ok(bytes) => content_hash(&bytes),
        Err(error) => {
            eprintln!(
                "[AetherDesk] failed to read default theme {}: {}",
                path.display(),
                error
            );
            return DefaultThemeOutcome::Kept;
        }
    };

    if current_hash == embedded_hash {
        tracked.insert(file_name.to_string(), current_hash);
        return DefaultThemeOutcome::Unchanged;
    }

    let is_tracked_seed = tracked
        .get(file_name)
        .is_some_and(|hash| *hash == current_hash);
    let is_legacy_seed = legacy
        .iter()
        .any(|(name, hash)| *name == file_name && *hash == current_hash);
    if !is_tracked_seed && !is_legacy_seed {
        return DefaultThemeOutcome::Kept;
    }

    match fs::write(&path, content) {
        Ok(()) => {
            tracked.insert(file_name.to_string(), embedded_hash);
            DefaultThemeOutcome::Refreshed
        }
        Err(error) => {
            eprintln!(
                "[AetherDesk] failed to refresh default theme {}: {}",
                path.display(),
                error
            );
            DefaultThemeOutcome::Kept
        }
    }
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
    let selected = resolve_selected(&themes_dir(), selected_file);
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
/// 3. otherwise `None`.
pub fn personal_wallpaper_path(selected_file: &str) -> Result<Option<PathBuf>, String> {
    let selected = resolve_selected(&wallpapers_dir(), selected_file);
    if let Some(path) = selected {
        return Ok(Some(path));
    }

    Ok(wallpaper_candidates()?.into_iter().next())
}

/// Lists image files inside `config/wallpapers/`, sorted by file name.
fn wallpaper_candidates() -> Result<Vec<PathBuf>, String> {
    list_files_with_extension(&wallpapers_dir(), "img")
}

/// Read the active wallpaper as a browser-ready data URI. Empty string when no
/// wallpaper exists. Data URIs avoid Tauri asset-protocol scope issues.
/// Refuses files over `MAX_WALLPAPER_BYTES`: the whole image is held in
/// memory plus ~33% Base64 overhead on every appearance refresh, so a huge
/// file would freeze the UI instead of theming it.
pub fn read_personal_wallpaper_data_uri(selected_file: &str) -> Result<Option<String>, String> {
    const MAX_WALLPAPER_BYTES: u64 = 8 * 1024 * 1024;
    let Some(path) = personal_wallpaper_path(selected_file)? else {
        return Ok(None);
    };
    let size = fs::metadata(&path)
        .map_err(|e| format!("Failed to read wallpaper {}: {}", path.display(), e))?
        .len();
    if size > MAX_WALLPAPER_BYTES {
        return Err(format!(
            "Wallpaper {} is too large ({} MiB, max 8 MiB). Pick a smaller image.",
            path.display(),
            size / (1024 * 1024)
        ));
    }
    let bytes = fs::read(&path)
        .map_err(|e| format!("Failed to read wallpaper {}: {}", path.display(), e))?;
    let mime = wallpaper_mime(&path);
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(format!("data:{mime};base64,{encoded}")))
}

/// Name of the file currently applied (selected or first detected), for UI
/// feedback. `None` when nothing is available.
pub fn active_wallpaper_name(selected_file: &str) -> Option<String> {
    personal_wallpaper_path(selected_file)
        .ok()
        .flatten()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
}

/// Name of the theme currently applied (selected or first detected), for UI
/// feedback. `None` when nothing is available.
pub fn active_theme_name(selected_file: &str) -> Option<String> {
    theme_path(selected_file).ok().flatten().and_then(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
    })
}

pub fn icon_path(selected_file: &str) -> Result<Option<PathBuf>, String> {
    let selected = resolve_selected(&icons_dir(), selected_file);
    if let Some(path) = selected {
        return Ok(Some(path));
    }
    Ok(icon_candidates()?.into_iter().next())
}

fn icon_candidates() -> Result<Vec<PathBuf>, String> {
    list_files_with_extension(&icons_dir(), "icon")
}

pub fn active_icon_name(selected_file: &str) -> Option<String> {
    icon_path(selected_file).ok().flatten().and_then(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
    })
}

fn icon_from_bytes(bytes: &[u8]) -> Result<tauri::image::Image<'static>, String> {
    let decoded =
        image::load_from_memory(bytes).map_err(|e| format!("Unsupported icon format: {}", e))?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err("Icon image is empty".to_string());
    }
    Ok(tauri::image::Image::new_owned(
        rgba.into_raw(),
        width,
        height,
    ))
}

pub fn load_window_icon(path: &Path) -> Result<tauri::image::Image<'static>, String> {
    let bytes =
        fs::read(path).map_err(|e| format!("Failed to read icon {}: {}", path.display(), e))?;
    icon_from_bytes(&bytes)
}

pub fn default_window_icon() -> Result<tauri::image::Image<'static>, String> {
    icon_from_bytes(DEFAULT_AETHER_ICON)
}

/// Applies the window icon only. Startup uses this variant because the shell
/// shortcut sync below spawns PowerShell and must not delay first paint.
pub fn apply_window_icon_only(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    let settings = crate::core::settings::load_settings(app);
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
    Ok(())
}

/// Syncs the shell shortcut icon and cache. Costly (PowerShell + shell cache),
/// so startup runs it on the deferred worker; saving an icon runs it inline.
pub fn sync_shell_shortcuts(app: &tauri::AppHandle) -> Result<(), String> {
    let settings = crate::core::settings::load_settings(app);
    let source = if settings.custom_icon_enabled {
        icon_path(&settings.icon_selected_file).ok().flatten()
    } else {
        None
    };
    let shell_ico = crate::core::shell_shortcuts::materialize_shell_ico(source.as_deref())?;
    crate::core::shell_shortcuts::sync_windows_shortcuts(&shell_ico);
    Ok(())
}

/// User-visible icon change: set the window icon and sync the shell now,
/// because the user expects the Start Menu entry to match immediately.
pub fn apply_window_icon(app: &tauri::AppHandle) -> Result<(), String> {
    apply_window_icon_only(app)?;
    sync_shell_shortcuts(app)
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
        fs::copy(&picked_path, &destination).map_err(|e| {
            format!(
                "Failed to copy {} into {}: {}",
                picked_path.display(),
                folder.display(),
                e
            )
        })?;
    }

    Ok(file_name.to_string())
}

/// Resolves a stored selection: only a single bare file name inside `folder`.
/// Rejects `..`, `/`, `\`, absolute paths and drive prefixes so a tampered
/// `settings.json` cannot escape `AetherData/config/...`. Otherwise `None`
/// (fall back to first detected file, as before).
fn resolve_selected(folder: &Path, selected_file: &str) -> Option<PathBuf> {
    let selected_file = selected_file.trim();
    if selected_file.is_empty() {
        return None;
    }
    if selected_file.contains(['/', '\\'])
        || Path::new(selected_file).is_absolute()
        || Path::new(selected_file)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let file_name = Path::new(selected_file).file_name()?.to_str()?;
    if file_name != selected_file {
        return None;
    }
    let candidate = folder.join(file_name);
    if !candidate.starts_with(folder) || !candidate.is_file() {
        return None;
    }
    Some(candidate)
}

/// Lists files inside `dir` filtered by `kind` ("css" or "img"), sorted by
/// file name. Missing folder → empty vec (not an error).
fn list_files_with_extension(dir: &Path, kind: &str) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries =
        fs::read_dir(dir).map_err(|e| format!("Failed to read folder {}: {}", dir.display(), e))?;
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
        // NOTE: no `svg` arm on purpose. `WALLPAPER_EXTENSIONS` does not accept
        // SVG (script-capable XML in the WebView is not worth the risk), so an
        // SVG extension falls through to `application/octet-stream` and is
        // never served as an image.
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn standard_base64_vectors_match_rfc_4648() {
        use base64::Engine;
        let encode = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode(&[0xff]), "/w==");
    }
}
