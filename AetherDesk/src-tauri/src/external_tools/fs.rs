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
            format!(
                "Failed to create directory {}: {}",
                parent.display(),
                error
            )
        })?;
    }

    fs::write(&temp_path, bytes).map_err(|error| {
        format!(
            "Failed to write temporary file {}: {}",
            temp_path.display(),
            error
        )
    })?;
    fs::rename(&temp_path, path).map_err(|error| {
        format!("Failed to finalize file {}: {}", path.display(), error)
    })
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
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// Reads a file for string scanning: whole when reasonably sized, otherwise
/// head + tail (where executable symbol strings typically live). Never more
/// than `HEAD_LIMIT + TAIL_LIMIT` bytes in memory per file.
pub fn read_for_scan(path: &Path) -> Result<Vec<u8>, String> {
    const FULL_LIMIT: u64 = 256 * 1024 * 1024; // 256 MiB
    const HEAD_LIMIT: u64 = 64 * 1024 * 1024;  // 64 MiB
    const TAIL_LIMIT: u64 = 16 * 1024 * 1024;  // 16 MiB

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
