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
