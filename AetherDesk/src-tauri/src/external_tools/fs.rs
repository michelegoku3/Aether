//! Utility filesystem condivise (scrittura atomica, walk ricorsivi, scansione).
//!
//! Estratte da `core/backup.rs` perché servono a più domini (backup crack,
//! deploy online, journal): un'unica implementazione, testata una volta sola.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Write bytes to `path` atomically via a temp file + rename.
///
/// Crea le directory mancanti del parent. Su Windows `std::fs::rename`
/// rimpiazza il file di destinazione esistente (MoveFileExW con
/// REPLACE_EXISTING), quindi la scrittura resta "tutto o niente" anche
/// quando il file target esiste già.
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

/// Tutti i file sotto `root` (ricorsivo, deterministico: ogni livello è
/// ordinato per nome). Non segue symlink e ignora silenziosamente le
/// directory illeggibili (best-effort, come gli scan di UCOnline2).
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
        // Ordinamento deterministico: i test e la detection dipendono
        // dall'ordine stabile dei risultati.
        dirs.sort();
        files.sort();
        out.extend(files);
        for dir in dirs.into_iter().rev() {
            stack.push(dir);
        }
    }

    out
}

/// Tutte le directory sotto `root` (ricorsivo, BFS: le directory di livello
/// più basso vengono prima di quelle più profonde). Deterministico, non
/// segue symlink.
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

/// True quando `haystack` contiene la sequenza `needle` (ricerca binaria su
/// byte, case-sensitive — equivalente a `findstr` senza `/i`).
pub fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// Legge un file per scansione di stringhe: intero se di dimensione
/// ragionevole, altrimenti inizio + coda (dove vivono tipicamente le
/// stringhe simboliche degli eseguibili). Mai più di `HEAD_LIMIT + TAIL_LIMIT`
/// byte in memoria per file.
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

    let head = HEAD_LIMIT as usize;
    let tail = TAIL_LIMIT as usize;
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
