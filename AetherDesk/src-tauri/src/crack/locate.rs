// Locating where each extracted crack file should land in the game.
//
// Crack archives are inconsistent: some place files directly at the game root,
// others nest them under wrapper folders that may or may not already exist in
// the game. Instead of guessing a single "crack root" (which fails when a
// folder like `Hk project` already exists in the game and must be merged, not
// descended into), we resolve each file **individually** by matching its path
// against the game's existing structure:
//
//   * keep the longest path suffix whose parent directory exists in the game
//     (this merges into existing folders such as `files/Hk project`), or
//   * otherwise strip wrapper components until the file fits at the game root.
//
// "The game root" here is the folder the game actually lives in. Steam's
// `installdir` is often a pure wrapper (`common/<name>/<name>/game.exe`): the
// installdir folder then holds nothing but the real game folder. A crack file
// the game does not already have belongs next to the game, not next to that
// wrapper, so `game_content_root` resolves the base once per call and only
// descends through a folder that is the installdir's only entry.
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Determine the target location in `base_root` for a staged file whose path
/// relative to the archive root is `archive_rel`.
///
/// The returned path is always `base_root.join(some_suffix)` and points into
/// an existing directory chain (or `base_root` itself) so that applied files
/// merge into the real game tree. Callers pass the game's real root (see
/// `game_content_root`).
pub fn resolve_file_target(archive_rel: &Path, base_root: &Path) -> PathBuf {
    let components: Vec<OsString> = archive_rel
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_os_string()),
            _ => None,
        })
        .collect();

    if components.is_empty() {
        return base_root.to_path_buf();
    }

    // Try the longest suffix first (full path), then strip one component at a
    // time. base_root is a directory, so the final iteration always matches and
    // returns the file at the game root.
    for start in 0..components.len() {
        let suffix: PathBuf = components[start..].iter().collect();
        let candidate = base_root.join(&suffix);
        if let Some(parent) = candidate.parent() {
            if parent.is_dir() {
                return candidate;
            }
        }
    }

    // Defensive fallback (unreachable in practice because base_root is a dir).
    base_root.join(archive_rel)
}

/// The folder the game's files actually live in.
///
/// Steam's `installdir` is often a pure wrapper: `common/<name>/<name>/game.exe`
/// holds nothing but the folder with the real game (exe, `<name>_Data`, DLLs).
/// A crack file the game does not already have must land next to the game, not
/// next to that wrapper.
///
/// The descent is deliberately conservative: it follows only a folder that is
/// the installdir's *only* entry. Anything else (another folder, a file, an
/// empty folder) means the game may genuinely live at the installdir level, so
/// the installdir itself stays the base. That also makes the change invisible
/// for every game that is not wrapper-packaged: the base is the same path.
///
/// One consequence is intentional: a file sitting next to the wrapper folder
/// keeps the base at the installdir, and that includes the crack files a build
/// older than this rule left there. Relocating them automatically would need a
/// way to tell a crack file from a file the game owns, which is a bigger
/// decision than resolving the content root; the recovery path is removing the
/// previous crack (the "remove applied crack" flow, or answering "choose a new
/// crack" in the saved-crack prompt) and applying again.
fn game_content_root(installdir: &Path) -> PathBuf {
    let mut current = installdir.to_path_buf();
    loop {
        let Ok(entries) = std::fs::read_dir(&current) else {
            return current;
        };
        let mut only_folder: Option<PathBuf> = None;
        for entry in entries.flatten() {
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            if !is_dir || only_folder.is_some() {
                return current;
            }
            only_folder = Some(entry.path());
        }
        match only_folder {
            Some(inner) => current = inner,
            // Empty folder: there is nothing to descend into.
            None => return current,
        }
    }
}

/// Determine all target locations for a staged file whose path relative to the
/// archive root is `archive_rel`, inside the game rooted at `game_root` (Steam's
/// `installdir` folder).
///
/// When `is_flat_crack` is true (meaning every file in the crack source is a plain
/// normal file without any folder structure, e.g. `steam_api64.dll`), we recursively
/// search `game_root` for any existing files whose name matches `archive_rel.file_name()`.
///   - If matching files are found in any subdirectory of `game_root`, all their
///     paths are returned so every matching file in the game is replaced.
///   - If no matching files exist in `game_root`, we fall back to placing the
///     file in the game's real root (`game_content_root`), as established.
///
/// When `is_flat_crack` is false (the crack archive contains folder structure),
/// it returns `[resolve_file_target(archive_rel, game_content_root)]`.
pub fn resolve_file_targets(
    archive_rel: &Path,
    game_root: &Path,
    is_flat_crack: bool,
) -> Vec<PathBuf> {
    // Files the game already has are replaced where they are (the search below
    // descends into wrapper folders too); files it does not have yet belong in
    // the folder that holds the game itself.
    let content_root = game_content_root(game_root);

    if is_flat_crack {
        if let Some(file_name) = archive_rel.file_name().and_then(|n| n.to_str()) {
            let mut matches = Vec::new();
            find_matching_files_recursive(game_root, file_name, &mut matches);
            if !matches.is_empty() {
                matches.sort();
                matches.dedup();
                return matches;
            }
        }
    }

    vec![resolve_file_target(archive_rel, &content_root)]
}

/// Recursively searches `dir` for any existing file whose name matches `target_name`
/// (case-insensitive).
fn find_matching_files_recursive(dir: &Path, target_name: &str, matches: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_matching_files_recursive(&path, target_name, matches);
        } else if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.eq_ignore_ascii_case(target_name) {
                    matches.push(path);
                }
            }
        }
    }
}
