// Local install engine.
//
// Owns the full "Local download" pipeline for one game:
//   stage each source (zip/rar/7z archive or loose file/folder) → route the
//   staged content into Steam:
//     * `*.lua`      → `<Steam>/config/stplug-in`   (Lua unlock files)
//     * `*.manifest` → `<Steam>/depotcache`         (depot decryption manifests)
//     * anything else → the game's folder in `steamapps/common/<game>`
//
// Lua/manifest files are also mirrored into the per-game
// `AetherData/backup/<app_id>/lua` folder, matching the central Lua-backup
// step of the online download pipeline.
//
// Like the crack engine, this module is deliberately Tauri-agnostic: it only
// needs the App ID, the game name, the Steam root, the optional active library
// and the list of source paths. The thin Tauri command in
// `commands/local.rs` is just a wrapper around it.
use crate::core::backup::GameBackup;
use crate::core::paths::LocalAppPaths;
use crate::crack::archive;
use crate::manifest::pins::LuaManifestPins;
use crate::steam::compat::SteamCompat;
use crate::steam::library::SteamLibraryScanner;
use std::fs;
use std::path::{Path, PathBuf};

/// Default password tried for password-protected archives (same as cracks).
const DEFAULT_ARCHIVE_PASSWORD: &str = "online-fix.me";

/// Human-readable report of a bulk local import run.
#[derive(Debug, Default)]
pub struct BulkInstallReport {
    pub sources: usize,
    pub lua_files: usize,
    pub manifest_files: usize,
    pub unique_apps: usize,
}

/// Bulk recursive importer for any combination of loose files, folders, and archives (.zip/.rar/.7z).
/// Searches recursively everywhere, identifies every .lua and .manifest file, and routes them to Steam.
pub fn install_bulk_local_pipeline(
    steam_path: &Path,
    sources: &[String],
    download_games_with_updates_on: bool,
) -> Result<BulkInstallReport, String> {
    if sources.is_empty() {
        return Err("No local files or folders selected.".to_string());
    }

    let steam = SteamCompat::new(steam_path.display().to_string());
    let plugin_dir = steam.get_plugin_dir();
    let depotcache_dir = steam.get_depotcache_dir();

    fs::create_dir_all(&plugin_dir)
        .map_err(|e| format!("Failed to create plugin dir {}: {}", plugin_dir.display(), e))?;
    fs::create_dir_all(&depotcache_dir)
        .map_err(|e| format!("Failed to create depotcache dir {}: {}", depotcache_dir.display(), e))?;

    let root_staging = LocalAppPaths::temp_dir().join(format!(
        "bulk_local_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&root_staging)
        .map_err(|e| format!("Failed to create temp staging {}: {}", root_staging.display(), e))?;

    let mut discovered_lua: Vec<PathBuf> = Vec::new();
    let mut discovered_manifests: Vec<PathBuf> = Vec::new();
    let mut counter: usize = 0;

    let result = (|| -> Result<(), String> {
        for source in sources {
            let path = PathBuf::from(source);
            if !path.exists() {
                continue;
            }
            explore_and_extract_recursive(
                &path,
                &root_staging,
                &mut discovered_lua,
                &mut discovered_manifests,
                &mut counter,
                0,
            )?;
        }
        Ok(())
    })();

    if let Err(e) = result {
        let _ = fs::remove_dir_all(&root_staging);
        return Err(e);
    }

    if discovered_lua.is_empty() && discovered_manifests.is_empty() {
        let _ = fs::remove_dir_all(&root_staging);
        return Err("No .lua or .manifest files were found in the selected files, folders, or archives.".to_string());
    }

    let mut report = BulkInstallReport {
        sources: sources.len(),
        ..BulkInstallReport::default()
    };
    let mut unique_apps = std::collections::HashSet::new();

    // Route .manifest files
    for manifest_path in &discovered_manifests {
        if let Some(file_name) = manifest_path.file_name() {
            let dest = depotcache_dir.join(file_name);
            if let Err(e) = fs::copy(manifest_path, &dest) {
                crate::desk_log_error!("local", "Failed to copy manifest {}: {}", manifest_path.display(), e);
            } else {
                report.manifest_files += 1;
            }
        }
    }

    // Route .lua files
    for lua_path in &discovered_lua {
        if let Some(file_name) = lua_path.file_name() {
            let stem = lua_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            let app_id_opt = app_id_from_lua_stem(&stem)
                .or_else(|| extract_app_id_from_lua_content(lua_path));

            let live_name = if let Some(app_id) = app_id_opt {
                unique_apps.insert(app_id);
                if let Ok(backup) = GameBackup::for_app(app_id) {
                    let backup_dir = backup.lua_dir();
                    let _ = fs::create_dir_all(&backup_dir);
                    let backup_dest = backup_dir.join(file_name);
                    let _ = fs::copy(lua_path, &backup_dest);
                }
                format!("{}.lua", app_id)
            } else {
                file_name.to_string_lossy().to_string()
            };

            let dest = plugin_dir.join(&live_name);
            if let Err(e) = fs::copy(lua_path, &dest) {
                crate::desk_log_error!("local", "Failed to copy lua {}: {}", lua_path.display(), e);
            } else {
                report.lua_files += 1;
                if let Some(app_id) = app_id_opt {
                    if download_games_with_updates_on {
                        let lua = LuaManifestPins::new(steam_path.to_path_buf(), app_id);
                        let _ = lua.set_updates_enabled(true);
                    }
                }
            }
        }
    }

    report.unique_apps = unique_apps.len();
    let _ = fs::remove_dir_all(&root_staging);

    Ok(report)
}

fn explore_and_extract_recursive(
    current: &Path,
    staging_root: &Path,
    lua_out: &mut Vec<PathBuf>,
    manifest_out: &mut Vec<PathBuf>,
    counter: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 10 {
        return Ok(());
    }

    if current.is_dir() {
        let entries = match fs::read_dir(current) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            explore_and_extract_recursive(
                &path,
                staging_root,
                lua_out,
                manifest_out,
                counter,
                depth + 1,
            )?;
        }
    } else if current.is_file() {
        let ext = current
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if ext == "lua" {
            lua_out.push(current.to_path_buf());
        } else if ext == "manifest" {
            manifest_out.push(current.to_path_buf());
        } else if matches!(ext.as_str(), "zip" | "rar" | "7z") {
            let sub_staging = staging_root.join(format!("extract_{}", *counter));
            *counter += 1;
            let _ = fs::create_dir_all(&sub_staging);
            if archive::stage_source(current, &sub_staging, DEFAULT_ARCHIVE_PASSWORD).is_ok() {
                explore_and_extract_recursive(
                    &sub_staging,
                    staging_root,
                    lua_out,
                    manifest_out,
                    counter,
                    depth + 1,
                )?;
            }
        }
    }

    Ok(())
}

fn extract_app_id_from_lua_content(path: &Path) -> Option<u32> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            continue;
        }
        for keyword in &["setManifestid", "setmanifestid", "addappid", "addAppId", "appid", "AppId"] {
            if let Some(pos) = trimmed.find(keyword) {
                let rest = &trimmed[pos + keyword.len()..];
                let digits: String = rest
                    .chars()
                    .skip_while(|c| !c.is_ascii_digit())
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(id) = digits.parse::<u32>() {
                    if id > 0 {
                        return Some(id);
                    }
                }
            }
        }
    }
    None
}

/// Human-readable summary of one local install run.
#[derive(Debug, Default)]
pub struct LocalInstallReport {
    /// Number of source files/folders processed.
    pub sources: usize,
    /// Number of files written into Steam (all destinations combined).
    pub applied: usize,
    /// Number of `.lua` files installed into `config/stplug-in`.
    pub lua_files: usize,
    /// Number of `.manifest` files installed into `depotcache`.
    pub manifest_files: usize,
    /// Absolute path of the Steam game folder used for regular game files.
    pub target: String,
    /// Steam-relative paths of the installed files (capped for display).
    pub files: Vec<String>,
}

/// Maximum number of relative paths kept in the report (the UI truncates the
/// message anyway; this avoids unbounded memory use on huge games).
const MAX_REPORT_FILES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalFileKind {
    Lua,
    Manifest,
    GameFile,
}

pub(crate) fn classify_local_file(path: &Path) -> LocalFileKind {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "lua" => LocalFileKind::Lua,
        "manifest" => LocalFileKind::Manifest,
        _ => LocalFileKind::GameFile,
    }
}

/// Run the local install pipeline for the given sources.
///
/// Lua and manifest files found anywhere in the staged content are routed to
/// their Steam system folders (`config/stplug-in` and `depotcache`); every
/// other file goes into the game's Steam install directory: if the game is
/// already installed (appmanifest present) its current folder is reused,
/// otherwise `<library>/steamapps/common/<game name>` is created in the active
/// library (or in the Steam root when no active library is configured).
pub fn install_local_pipeline(
    app_id: u32,
    game_name: &str,
    steam_path: &Path,
    active_library: Option<&str>,
    sources: &[String],
) -> Result<LocalInstallReport, String> {
    if sources.is_empty() {
        crate::desk_log_info!("local", "Local install aborted for AppID {}: no source files selected", app_id);
        return Err("No local files selected.".to_string());
    }

    crate::desk_log_info!("local", "Local install pipeline started for AppID {} ({}): {} source(s), steam root {}, active library: {}",
        app_id,
        game_name,
        sources.len(),
        steam_path.display(),
        active_library.unwrap_or("<not set>"));

    let game_dir = resolve_target_dir(app_id, game_name, steam_path, active_library);
    crate::desk_log_info!("local", "Resolved game folder for AppID {}: {}", app_id, game_dir.display());
    // Note: the game folder is created lazily by `install_staged_tree` only
    // when a real game file is written, so a sourceset containing only
    // lua/manifest files does not leave an empty folder behind.

    // Steam system folders for routed files (same locations used by the
    // online download pipeline: Lua configs and depot manifests).
    let steam = SteamCompat::new(steam_path.display().to_string());
    let plugin_dir = steam.get_plugin_dir();
    let depotcache_dir = steam.get_depotcache_dir();
    crate::desk_log_info!("local", "Routing folders: lua → {}, manifest → {}", plugin_dir.display(), depotcache_dir.display());

    let backup = GameBackup::for_app(app_id)?;
    let lua_backup_dir = backup.lua_dir();
    let staging = create_local_staging(app_id)?;
    crate::desk_log_info!("local", "Staging folder created: {} (lua backup: {})", staging.display(), lua_backup_dir.display());

    let mut report = LocalInstallReport {
        target: game_dir.display().to_string(),
        ..LocalInstallReport::default()
    };

    // Ensure staging is cleaned up even when a source fails mid-way.
    let result = (|| -> Result<(), String> {
        for source in sources {
            report.sources += 1;
            let source_path = PathBuf::from(source.as_str());
            crate::desk_log_info!("local", "Processing source {}/{}: {}", report.sources, sources.len(), source_path.display());

            if !source_path.exists() {
                crate::desk_log_error!("local", "Local source not found: {}", source_path.display());
                return Err(format!("Local file not found: {}", source_path.display()));
            }

            if source_path.is_dir() {
                // A dropped folder is staged as-is, preserving its structure.
                crate::desk_log_info!("local", "Source is a folder, staging its contents as-is: {}", source_path.display());
                copy_dir_contents(&source_path, &staging)?;
            } else {
                // Archives (.zip/.rar/.7z) are extracted; anything else is
                // staged as a loose file by the shared archive helper.
                let extension = source_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(extension.as_str(), "zip" | "rar" | "7z") {
                    crate::desk_log_info!("local", "Source is a .{} archive, extracting into staging", extension);
                } else {
                    crate::desk_log_info!("local", "Source is a loose file, staging it as-is (extension: {})",
                        if extension.is_empty() { "<none>" } else { extension.as_str() });
                }
                archive::stage_source(&source_path, &staging, DEFAULT_ARCHIVE_PASSWORD)?;
            }

            // Validate before touching Steam: every staged .lua must belong to
            // the selected game, otherwise the whole source is rejected and
            // nothing is copied (no partial installs).
            validate_staged_lua_files(&staging, app_id)?;

            // Route the staged tree into the right Steam locations.
            install_staged_tree(
                app_id,
                &staging,
                &staging,
                &game_dir,
                &plugin_dir,
                &depotcache_dir,
                &lua_backup_dir,
                &mut report,
            )?;
            crate::desk_log_info!("local", "Source {} routed into Steam (running totals: {} file(s), {} lua, {} manifest)",
                source_path.display(), report.applied, report.lua_files, report.manifest_files);

            // Clear staged files before processing the next source.
            archive::clear_staging_contents(&staging)?;
        }
        Ok(())
    })();

    // Best-effort cleanup regardless of success/error.
    let _ = archive::remove_staging(&staging);
    crate::desk_log_info!("local", "Staging folder cleaned up: {}", staging.display());

    if let Err(error) = &result {
        crate::desk_log_error!("local", "Local install pipeline failed for AppID {}: {}", app_id, error);
    }

    result?;
    Ok(report)
}

/// Reads the leading numeric AppID from provider-style Lua names. Supports the
/// canonical `<appid>.lua` and build-labelled `<appid>_<buildid>.lua` forms
/// without accepting an AppID that merely appears later in an unrelated name.
pub(crate) fn app_id_from_lua_stem(stem: &str) -> Option<u32> {
    let digit_count = stem
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }
    stem[..digit_count].parse().ok()
}

/// Recursively validate every staged `.lua` file: its leading App ID must
/// match the selected game. A Lua for a different App ID means the archive
/// belongs to a different game, so the source is rejected before Steam is
/// touched. Build-labelled names are canonicalized during routing.
fn validate_staged_lua_files(current: &Path, app_id: u32) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|error| format!("Failed to read folder {}: {}", current.display(), error))?
    {
        let entry = entry.map_err(|error| format!("Failed to read entry: {}", error))?;
        let path = entry.path();

        if path.is_dir() {
            validate_staged_lua_files(&path, app_id)?;
            continue;
        }

        let is_lua = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("lua"))
            .unwrap_or(false);
        if !is_lua {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();

        match app_id_from_lua_stem(&stem) {
            Some(lua_app_id) if lua_app_id == app_id => {
                crate::desk_log_info!(
                    "local",
                    "Lua file {} validated: leading App ID matches the selected game ({})",
                    file_name,
                    app_id
                );
            }
            Some(lua_app_id) => {
                crate::desk_log_error!("local", "Lua file {} belongs to App ID {} but the selected game is App ID {}: refusing to install", file_name, lua_app_id, app_id);
                return Err(format!(
                    "The Lua file {} belongs to App ID {}, but the selected game is App ID {}. They are not the same game: installation aborted.",
                    file_name, lua_app_id, app_id
                ));
            }
            None => {
                crate::desk_log_error!("local", "Lua file {} has no leading numeric App ID: refusing to install", file_name);
                return Err(format!(
                    "The Lua file {} does not start with a Steam App ID, so it cannot be verified against the selected game. Accepted names include <appid>.lua and <appid>_<buildid>.lua.",
                    file_name
                ));
            }
        }
    }

    Ok(())
}

/// Recursively walk the staged tree and copy every file to its destination:
/// `.lua` files into `config/stplug-in`, `.manifest` files into `depotcache`
/// (both flattened by file name, as Steam expects), everything else into the
/// game folder preserving its relative structure. Lua/manifest files are also
/// mirrored into the per-game `lua` backup folder in AetherData, matching the
/// central Lua-backup step of the online download pipeline. Existing files are
/// overwritten.
fn install_staged_tree(
    app_id: u32,
    root: &Path,
    current: &Path,
    game_dir: &Path,
    plugin_dir: &Path,
    depotcache_dir: &Path,
    lua_backup_dir: &Path,
    report: &mut LocalInstallReport,
) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|error| format!("Failed to read folder {}: {}", current.display(), error))?
    {
        let entry = entry.map_err(|error| format!("Failed to read entry: {}", error))?;
        let path = entry.path();

        if path.is_dir() {
            install_staged_tree(
                app_id,
                root,
                &path,
                game_dir,
                plugin_dir,
                depotcache_dir,
                lua_backup_dir,
                report,
            )?;
            continue;
        }

        let file_kind = classify_local_file(&path);
        let file_name = entry.file_name();

        let (dest, display_path, also_backup) = match file_kind {
            LocalFileKind::Lua => {
                report.lua_files += 1;
                // Steam loads only <appid>.lua. Provider files may preserve a
                // build suffix (<appid>_<buildid>.lua); validation above proves
                // the leading AppID belongs to the selected game, then routing
                // canonicalizes the live filename while backup keeps the source.
                let live_name = format!("{}.lua", app_id);
                crate::desk_log_info!(
                    "local",
                    "Routing lua file {} → config/stplug-in/{}",
                    file_name.to_string_lossy(),
                    live_name
                );
                (
                    plugin_dir.join(&live_name),
                    format!("config/stplug-in/{}", live_name),
                    true,
                )
            }
            LocalFileKind::Manifest => {
                report.manifest_files += 1;
                crate::desk_log_info!("local", "Routing manifest file {} → depotcache", file_name.to_string_lossy());
                (
                    depotcache_dir.join(&file_name),
                    format!("depotcache/{}", file_name.to_string_lossy()),
                    true,
                )
            }
            LocalFileKind::GameFile => {
                let relative = path
                    .strip_prefix(root)
                    .map(|rel| rel.to_path_buf())
                    .unwrap_or_else(|_| PathBuf::from(&file_name));
                crate::desk_log_info!("local", "Routing game file {} → {}", relative.display(), game_dir.display());
                (
                    game_dir.join(&relative),
                    format!("game/{}", relative.display()),
                    false,
                )
            }
        };

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create folder {}: {}", parent.display(), error)
            })?;
        }
        fs::copy(&path, &dest).map_err(|error| {
            format!(
                "Failed to copy file {} to {}: {}",
                path.display(),
                dest.display(),
                error
            )
        })?;

        // Keep the per-game lua backup in sync (same role as
        // `GameBackup::backup_lua_artifacts` for online downloads).
        if also_backup {
            let backup_dest = lua_backup_dir.join(&file_name);
            fs::copy(&path, &backup_dest).map_err(|error| {
                format!(
                    "Failed to back up file {} to {}: {}",
                    path.display(),
                    backup_dest.display(),
                    error
                )
            })?;
            crate::desk_log_info!("local", "Mirrored {} into AetherData lua backup", file_name.to_string_lossy());
        }

        report.applied += 1;
        if report.files.len() < MAX_REPORT_FILES {
            report.files.push(display_path);
        }
    }

    Ok(())
}

/// Resolve the Steam folder the local game content must be installed into.
fn resolve_target_dir(
    app_id: u32,
    game_name: &str,
    steam_path: &Path,
    active_library: Option<&str>,
) -> PathBuf {
    // If the game already has an appmanifest, install into its current folder
    // (whichever library it lives in).
    let scanner = SteamLibraryScanner::new(
        steam_path.to_path_buf(),
        active_library.map(|value| value.to_string()),
    );
    if let Some(game) = scanner
        .scan_installed_games()
        .into_iter()
        .find(|game| game.id == app_id)
    {
        if game.installed && !game.game_path.trim().is_empty() {
            let existing = PathBuf::from(&game.game_path);
            if existing.is_dir() {
                crate::desk_log_info!("local", "AppID {} is already installed, reusing its folder: {}", app_id, existing.display());
                return existing;
            }
            crate::desk_log_info!("local", "AppID {} has an appmanifest but its folder is missing: {}", app_id, existing.display());
        }
    }

    // Otherwise create the folder in the active library (falling back to the
    // Steam root) under steamapps/common/<game name>.
    let library = active_library
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| steam_path.to_path_buf());

    let resolved = library
        .join("steamapps")
        .join("common")
        .join(sanitize_folder_name(game_name, app_id));

    crate::desk_log_info!("local", "AppID {} is not installed, target folder will be {} (library: {})",
        app_id, resolved.display(), library.display());

    resolved
}

/// Turn a game name into a valid Windows folder name.
fn sanitize_folder_name(game_name: &str, app_id: u32) -> String {
    let sanitized: String = game_name
        .chars()
        .filter(|ch| !matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .collect();
    let sanitized = sanitized.trim().trim_end_matches(['.', ' ']).trim().to_string();
    if sanitized.is_empty() {
        crate::desk_log_info!("local", "Game name sanitizes to an empty folder name, falling back to app_{}", app_id);
        format!("app_{}", app_id)
    } else {
        sanitized
    }
}

/// Create a unique staging directory for a local install run.
///
/// Lives under `AetherData/temp` (like the crack staging) so Defender
/// exclusions on AetherData also cover local installs.
fn create_local_staging(app_id: u32) -> Result<PathBuf, String> {
    let staging = LocalAppPaths::temp_dir().join(format!(
        "local_{}_{}",
        app_id,
        std::process::id()
    ));
    fs::create_dir_all(&staging).map_err(|error| {
        format!(
            "Failed to create staging folder {}: {}",
            staging.display(),
            error
        )
    })?;
    Ok(staging)
}

/// Recursively copy the contents of `src` into `dest`, overwriting existing
/// files.
fn copy_dir_contents(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|error| {
        format!(
            "Failed to create folder {}: {}",
            dest.display(),
            error
        )
    })?;

    for entry in fs::read_dir(src)
        .map_err(|error| format!("Failed to read folder {}: {}", src.display(), error))?
    {
        let entry = entry.map_err(|error| format!("Failed to read entry: {}", error))?;
        let path = entry.path();
        let target = dest.join(entry.file_name());

        if path.is_dir() {
            copy_dir_contents(&path, &target)?;
        } else {
            fs::copy(&path, &target).map_err(|error| {
                format!(
                    "Failed to copy file {} to {}: {}",
                    path.display(),
                    target.display(),
                    error
                )
            })?;
        }
    }

    Ok(())
}
