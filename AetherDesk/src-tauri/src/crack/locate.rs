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
// `installdir` is often a wrapper (`common/<name>/<name>/game.exe`): the
// installdir folder then holds nothing but the real game folder. A crack file
// the game does not already have belongs next to the game, not next to that
// wrapper, so `game_content_root` resolves the base once per call: it descends
// while the installdir looks like a wrapper, that is while it holds a single
// folder, no executable of its own, and either no other entry at all or a
// folder named like the installdir (or holding its executable).
use std::ffi::{OsStr, OsString};
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
/// Steam's `installdir` is often a wrapper: `common/<name>/<name>/game.exe`
/// holds nothing but the folder with the real game (exe, `<name>_Data`, DLLs).
/// A crack file the game does not already have must land next to the game, not
/// next to that wrapper.
///
/// The descent follows a folder only while the installdir does not look like the
/// game itself (no executable at its level) and the folder does:
///
///   * it is the installdir's only entry (a pure wrapper, whatever its name), or
///   * it is the installdir's only folder and is named like the installdir (the
///     double-folder packaging convention) or holds an executable named like the
///     installdir.
///
/// Anything else - several folders, no folder, an executable at the installdir
/// level, or a folder carrying none of those signals - keeps the installdir as
/// the base. That also makes the change invisible for every game that is not
/// wrapper-packaged: the base is then the same path.
///
/// The residual gap is a stray file (a repack note, or a crack file left by a
/// build older than this rule) next to a folder with no signal at all. That shape
/// keeps the base at the installdir; the recovery is removing the previous crack
/// (the "remove applied crack" flow, or answering "choose a new crack" in the
/// saved-crack prompt) and applying again.
fn game_content_root(installdir: &Path) -> PathBuf {
    let expected_name = installdir
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    let mut current = installdir.to_path_buf();
    loop {
        let Ok(entries) = std::fs::read_dir(&current) else {
            return current;
        };
        let mut entries_seen = 0usize;
        let mut folders: Vec<PathBuf> = Vec::new();
        let mut holds_own_executable = false;
        for entry in entries.flatten() {
            entries_seen += 1;
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                folders.push(entry.path());
            } else if is_executable(&entry.path()) {
                holds_own_executable = true;
            }
        }

        // A pure wrapper: nothing but one folder, whatever it is called.
        if entries_seen == 1 && folders.len() == 1 {
            if let Some(folder) = folders.pop() {
                current = folder;
                continue;
            }
        }

        // The only folder, next to files the game does not own: it is the game
        // root when it carries one of the two signals. A symlink or junction
        // never reaches here: `file_type` does not follow reparse points, so it
        // is neither a folder nor an executable and the descent stops - which is
        // what we want, because the canonicalized target could leave the game
        // root and break every recorded path.
        if !holds_own_executable && folders.len() == 1 {
            if let Some(folder) = folders.pop() {
                let named_like_game = folder
                    .file_name()
                    .and_then(OsStr::to_str)
                    .map(|name| name.eq_ignore_ascii_case(expected_name))
                    .unwrap_or(false);
                if !expected_name.is_empty()
                    && (named_like_game || holds_executable_named(&folder, expected_name))
                {
                    current = folder;
                    continue;
                }
            }
        }

        return current;
    }
}

fn is_executable(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .map(|extension| extension.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

/// True when `folder` directly holds an executable file whose stem is
/// `expected_name` (case-insensitive). Links count as neither: `file_type` does
/// not follow reparse points.
fn holds_executable_named(folder: &Path, expected_name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries.flatten().any(|entry| {
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            return false;
        }
        let path = entry.path();
        is_executable(&path)
            && path
                .file_stem()
                .and_then(OsStr::to_str)
                .map(|stem| stem.eq_ignore_ascii_case(expected_name))
                .unwrap_or(false)
    })
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
        if let Some(file_name) = archive_rel.file_name() {
            let mut matches = Vec::new();
            find_matching_files(game_root, file_name, &mut matches);
            if !matches.is_empty() {
                matches.sort();
                matches.dedup();
                return matches;
            }
        }
    }

    vec![resolve_file_target(archive_rel, &content_root)]
}

/// Recursively finds every existing file named `file_name` under `dir`.
///
/// Delegates the name comparison to the filesystem (`dir.join(name).is_file()`)
/// instead of folding names in Rust: Windows matches case-insensitively with
/// Unicode (not ASCII) rules, so a file differing only by the case of a
/// non-ASCII letter — which ASCII folding missed — is now found.
fn find_matching_files(dir: &Path, file_name: &OsStr, matches: &mut Vec<PathBuf>) {
    let candidate = dir.join(file_name);
    if candidate.is_file() {
        matches.push(candidate);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            find_matching_files(&entry.path(), file_name, matches);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn flat_crack_without_a_match_falls_back_to_the_content_root() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("game");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("game.exe"), b"MZ").unwrap();

        let targets = resolve_file_targets(Path::new("newfile.dll"), &game, true);

        assert_eq!(targets, vec![game.join("newfile.dll")]);
    }

    #[cfg(windows)]
    #[test]
    fn flat_crack_replaces_an_existing_file_that_differs_only_by_non_ascii_case() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("Binaries")).unwrap();
        fs::write(game.join("game.exe"), b"MZ").unwrap();
        fs::write(game.join("Binaries/Файл.dll"), b"original").unwrap();

        // Windows treats these two names as the same file: the flat crack must
        // replace the existing one instead of dropping a copy at the root.
        let targets = resolve_file_targets(Path::new("файл.dll"), &game, true);

        assert_eq!(
            targets,
            vec![game.join("Binaries/файл.dll")],
            "the existing file must be matched case-insensitively"
        );
    }

    #[test]
    fn structured_crack_still_resolves_into_existing_folders() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("Binaries")).unwrap();
        fs::write(game.join("game.exe"), b"MZ").unwrap();

        let targets = resolve_file_targets(Path::new("Binaries/extra.dll"), &game, false);

        assert_eq!(targets, vec![game.join("Binaries/extra.dll")]);
    }
}
