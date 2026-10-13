//! Generic, reusable filesystem commands (opening folders in the file manager).
//!
//! Kept apart from domain commands (e.g. `custom_css`) for high cohesion:
//! each module has a single responsibility and domain commands must not
//! expose generic fs infrastructure.
//!
//! The path comes from the frontend, so it is allowlisted: only folders under
//! known roots (app data, install folder, Steam and its discovered libraries)
//! may be created/opened. Anything else is refused with a readable error,
//! never executed.
use crate::core::paths::LocalAppPaths;
use std::path::{Component, Path, PathBuf};

/// Generic, reusable command: opens any folder in the system file manager.
/// Used by the Online panel (game exe/DLL folders) and by the Settings view
/// (themes, wallpapers, icons folders).
#[tauri::command]
pub fn reveal_in_file_manager(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let folder = allowed_folder(&app, &path)?;
    crate::desk_log_info!("fs", "Opening folder in OS file manager: {}", folder.display());
    open_folder_in_explorer(&folder)
}

/// Resolve `raw` against the allowlisted roots. Returns the folder to open.
///
/// Existing folders are canonicalized (symlinks resolved) before the check;
/// missing folders are validated through their first existing ancestor so a
/// `..` escape can never sneak past the prefix comparison.
fn allowed_folder(app: &tauri::AppHandle, raw: &str) -> Result<PathBuf, String> {
    let roots = allowed_roots(app);
    let candidate = PathBuf::from(raw.trim());
    if !path_is_allowed(&candidate, &roots) {
        return Err("Refusing to open a folder outside the app data, install, or Steam library folders.".to_string());
    }
    Ok(candidate)
}

/// Roots the file manager may ever touch: app data + install folder (themes,
/// wallpapers, icons, logs) and the configured Steam tree (main install +
/// every extra library from `libraryfolders.vdf`, covering game exe/DLL
/// folders shown by the Online panel).
fn allowed_roots(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut roots = vec![
        LocalAppPaths::data_root(),
        LocalAppPaths::install_root(),
    ];
    let settings = crate::core::settings::load_settings(app);
    if let Ok(steam_root) = crate::steam::resolve::resolve_steam_path(&settings.steam_path) {
        let scanner = crate::steam::library::SteamLibraryScanner::new(steam_root);
        roots.extend(scanner.discover_library_paths());
    }
    roots
}

/// True when `candidate` stays inside one of `roots`. Component-wise prefix
/// comparison: `/foo` never matches `/foobar`, and `..` / absolute second
/// segments cannot escape because every `..` pops a real component first.
fn path_is_allowed(candidate: &Path, roots: &[PathBuf]) -> bool {
    // Walk up to the first existing ancestor for the canonical check; a
    // non-existent tail is validated lexically below.
    let mut existing = candidate;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                existing = parent;
            }
            _ => return false,
        }
    }
    let Ok(canonical_base) = existing.canonicalize() else {
        return false;
    };
    // The missing tail must be a plain descent: no `..`, no absolute parts.
    for part in tail.iter().rev() {
        let part_path = Path::new(part);
        if part_path.components().any(|c| !matches!(c, Component::Normal(_))) {
            return false;
        }
    }
    let mut reassembled = canonical_base;
    for part in tail.iter().rev() {
        reassembled.push(part);
    }
    roots.iter().any(|root| {
        let canonical_root = root
            .canonicalize()
            .unwrap_or_else(|_| root.clone());
        reassembled.starts_with(&canonical_root)
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn roots_in(temp: &std::path::Path) -> Vec<PathBuf> {
        let root = temp.join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        vec![root]
    }

    #[test]
    fn only_folders_inside_roots_are_allowed() {
        let temp = tempfile::tempdir().unwrap();
        let roots = roots_in(temp.path());
        let root = &roots[0];

        // Inside (existing + missing tail) is allowed.
        assert!(path_is_allowed(&root.join("sub"), &roots));
        assert!(path_is_allowed(&root.join("sub").join("new").join("dir"), &roots));
        // Outside, `..` escapes and prefix-sibling lookalikes are refused.
        assert!(!path_is_allowed(temp.path(), &roots));
        assert!(!path_is_allowed(&root.join("..").join("other"), &roots));
        assert!(!path_is_allowed(
            &PathBuf::from(format!("{}-sibling", root.display())),
            &roots
        ));
        assert!(!path_is_allowed(Path::new(""), &roots));
    }
}
