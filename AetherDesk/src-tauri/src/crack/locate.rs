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
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Determine the target location in `game_root` for a staged file whose path
/// relative to the archive root is `archive_rel`.
///
/// The returned path is always `game_root.join(some_suffix)` and points into
/// an existing directory chain (or the game root itself) so that applied files
/// merge into the real game tree.
pub fn resolve_file_target(archive_rel: &Path, game_root: &Path) -> PathBuf {
    let components: Vec<OsString> = archive_rel
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_os_string()),
            _ => None,
        })
        .collect();

    if components.is_empty() {
        return game_root.to_path_buf();
    }

    // Try the longest suffix first (full path), then strip one component at a
    // time. game_root is a directory, so the final iteration always matches and
    // returns the file at the game root.
    for start in 0..components.len() {
        let suffix: PathBuf = components[start..].iter().collect();
        let candidate = game_root.join(&suffix);
        if let Some(parent) = candidate.parent() {
            if parent.is_dir() {
                return candidate;
            }
        }
    }

    // Defensive fallback (unreachable in practice because game_root is a dir).
    game_root.join(archive_rel)
}

/// Determine all target locations in `game_root` for a staged file whose path
/// relative to the archive root is `archive_rel`.
///
/// When `is_flat_crack` is true (meaning every file in the crack source is a plain
/// normal file without any folder structure, e.g. `steam_api64.dll`), we recursively
/// search `game_root` for any existing files whose name matches `archive_rel.file_name()`.
///   - If matching files are found in any subdirectory of `game_root`, all their
///     paths are returned so every matching file in the game is replaced.
///   - If no matching files exist in `game_root`, we fall back to placing the
///     file at the game root (`[game_root.join(archive_rel)]`), as established.
///
/// When `is_flat_crack` is false (the crack archive contains folder structure),
/// it returns `[resolve_file_target(archive_rel, game_root)]`.
pub fn resolve_file_targets(
    archive_rel: &Path,
    game_root: &Path,
    is_flat_crack: bool,
) -> Vec<PathBuf> {
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

    vec![resolve_file_target(archive_rel, game_root)]
}

/// Recursively searches `dir` for any existing file whose name matches `target_name`
/// (case-insensitive).
fn find_matching_files_recursive(dir: &Path, target_name: &str, matches: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
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
