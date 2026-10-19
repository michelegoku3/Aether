//! Shared filesystem utilities (atomic writes, recursive walks, scanning).
//!
//! Extracted from `core/backup.rs` because several domains need them (crack
//! backup, online deploy, journal): a single implementation, tested once.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Write bytes to `path` atomically via a temp file + rename.
///
/// Creates missing parent directories. On Windows `std::fs::rename`
/// replaces the existing destination file (MoveFileExW with
/// REPLACE_EXISTING), so the write stays "all or nothing" even
/// when the target file already exists.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp_path = PathBuf::from(format!("{}.tmp", path.display()));

    if let Some(parent) = temp_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!("Failed to create directory {}: {}", parent.display(), error)
        })?;
    }

    fs::write(&temp_path, bytes).map_err(|error| {
        format!(
            "Failed to write temporary file {}: {}",
            temp_path.display(),
            error
        )
    })?;
    fs::rename(&temp_path, path)
        .map_err(|error| format!("Failed to finalize file {}: {}", path.display(), error))
}

/// Every file under `root` (recursive, deterministic: each level is
/// name-sorted). Does not follow symlinks and silently skips unreadable
/// directories (best-effort, like the UCOnline2 scans).
pub fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(path),
                Ok(t) if t.is_file() => files.push(path),
                _ => {}
            }
        }
        // Deterministic ordering: tests and detection depend on stable result order.
        dirs.sort();
        files.sort();
        out.extend(files);
        for dir in dirs.into_iter().rev() {
            stack.push(dir);
        }
    }

    out
}

/// All directories under `root` (recursive, BFS: shallower directories come
/// before deeper ones). Deterministic, does not follow symlinks.
pub fn walk_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);

    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                dirs.push(path);
            }
        }
        dirs.sort();
        for sub in dirs {
            out.push(sub.clone());
            queue.push_back(sub);
        }
    }

    out
}

/// True when `haystack` contains the `needle` sequence (byte-wise binary
/// search, case-sensitive — equivalent to `findstr` without `/i`).
pub fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Reads a file for string scanning: whole when reasonably sized, otherwise
/// head + tail (where executable symbol strings typically live). Never more
/// than `HEAD_LIMIT + TAIL_LIMIT` bytes in memory per file.
pub fn read_for_scan(path: &Path) -> Result<Vec<u8>, String> {
    const FULL_LIMIT: u64 = 256 * 1024 * 1024; // 256 MiB
    const HEAD_LIMIT: u64 = 64 * 1024 * 1024; // 64 MiB
    const TAIL_LIMIT: u64 = 16 * 1024 * 1024; // 16 MiB

    let size = fs::metadata(path)
        .map_err(|error| format!("Failed to stat {}: {}", path.display(), error))?
        .len();

    if size <= FULL_LIMIT {
        return fs::read(path)
            .map_err(|error| format!("Failed to read {}: {}", path.display(), error));
    }

    let head = usize::try_from(HEAD_LIMIT).expect("head limit fits in usize");
    let tail = usize::try_from(TAIL_LIMIT).expect("tail limit fits in usize");
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Failed to open {}: {}", path.display(), error))?;

    let mut buf = vec![0u8; head + tail];
    file.read_exact(&mut buf[..head])
        .map_err(|error| format!("Failed to read head of {}: {}", path.display(), error))?;
    file.seek(SeekFrom::End(-(TAIL_LIMIT as i64)))
        .map_err(|error| format!("Failed to seek in {}: {}", path.display(), error))?;
    file.read_exact(&mut buf[head..])
        .map_err(|error| format!("Failed to read tail of {}: {}", path.display(), error))?;

    Ok(buf)
}

/// Moves a file or directory, falling back to copy + delete when the source
/// and destination are on different volumes.
///
/// `fs::rename` is atomic and preferred; Windows returns
/// `ERROR_NOT_SAME_DEVICE` (surfaced as `ErrorKind::CrossesDevices`) when the
/// paths live on different drives, which is common: AetherDesk may be on `C:`
/// while a Steam library is on `D:`.
pub fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_then_remove(from, to)
        }
        Err(error) => Err(format!(
            "Failed to move {} -> {}: {error}",
            from.display(),
            to.display()
        )),
    }
}

/// Non-atomic fallback for `move_path`: copy the whole tree, then remove the
/// source. A crash between the two leaves both copies, which callers must
/// treat as "already moved" when the destination content matches.
fn copy_then_remove(from: &Path, to: &Path) -> Result<(), String> {
    copy_path(from, to)?;
    remove_path(from)
}

fn copy_path(from: &Path, to: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(from)
        .map_err(|error| format!("Failed to inspect {}: {error}", from.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("Refusing to copy linked path {}", from.display()));
    }
    if metadata.is_dir() {
        fs::create_dir_all(to)
            .map_err(|error| format!("Failed to create directory {}: {error}", to.display()))?;
        for entry in fs::read_dir(from)
            .map_err(|error| format!("Failed to read {}: {error}", from.display()))?
        {
            let entry =
                entry.map_err(|error| format!("Failed to read {}: {error}", from.display()))?;
            copy_path(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        fs::copy(from, to).map_err(|error| {
            format!(
                "Failed to copy {} -> {}: {error}",
                from.display(),
                to.display()
            )
        })?;
    } else {
        return Err(format!("Refusing to copy special file {}", from.display()));
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("Failed to remove directory {}: {error}", path.display()))
    } else {
        fs::remove_file(path)
            .map_err(|error| format!("Failed to remove {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_path_moves_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from.bin");
        let to = dir.path().join("nested/to.bin");
        fs::write(&from, b"payload").unwrap();

        move_path(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), b"payload");
    }

    #[test]
    fn move_path_moves_a_directory_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("plugins");
        fs::create_dir_all(from.join("nested")).unwrap();
        fs::write(from.join("a.dll"), b"a").unwrap();
        fs::write(from.join("nested/b.dll"), b"b").unwrap();
        let to = dir.path().join("backup/plugins");

        move_path(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(fs::read(to.join("a.dll")).unwrap(), b"a");
        assert_eq!(fs::read(to.join("nested/b.dll")).unwrap(), b"b");
    }

    #[test]
    fn copy_then_remove_is_the_cross_device_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        fs::create_dir_all(from.join("sub")).unwrap();
        fs::write(from.join("file"), b"data").unwrap();
        fs::write(from.join("sub/nested"), b"nested").unwrap();
        let to = dir.path().join("to");

        copy_then_remove(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(fs::read(to.join("file")).unwrap(), b"data");
        assert_eq!(fs::read(to.join("sub/nested")).unwrap(), b"nested");
    }
}
