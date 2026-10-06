//! Durable same-directory publication. No shared `.tmp`, no delete-before-rename.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("State path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("State directory: {e}"))?;
    let name = path
        .file_name()
        .ok_or("State path has no filename")?
        .to_string_lossy();
    let (temp, mut file) = loop {
        let temp = parent.join(format!(
            ".{name}.{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("State staging: {e}")),
        }
    };
    let result = (|| {
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("State flush: {e}"))?;
        drop(file);
        fs::rename(&temp, path).map_err(|e| format!("State commit {}: {e}", path.display()))?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("State directory flush: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Advisory lock shared by cooperating Desk processes. The OS releases it
/// on crash; the small sidecar may remain and is NOT a stale-lock sentinel.
pub fn lock(path: &Path) -> Result<std::fs::File, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("Cannot open state lock {}: {e}", path.display()))?;
    file.lock()
        .map_err(|e| format!("Cannot lock state {}: {e}", path.display()))?;
    Ok(file)
}

pub fn write_if_unchanged(path: &Path, expected: &[u8], bytes: &[u8]) -> Result<(), String> {
    if fs::read(path).map_err(|e| format!("Cannot recheck {}: {e}", path.display()))? != expected {
        crate::desk_log_warn!(
            "mutations",
            "External file conflict at {}; publication refused",
            path.display()
        );
        return Err(format!(
            "GAME_STATE_CONFLICT: {} changed externally; retry",
            path.display()
        ));
    }
    write_atomic(path, bytes)
}

/// Reserve a directory for ONE operation, including same-app runs on another
/// root. Never reuse a stale directory from an interrupted operation.
pub fn create_staging(parent: &Path, prefix: &str) -> Result<std::path::PathBuf, String> {
    fs::create_dir_all(parent).map_err(|e| format!("Staging parent: {e}"))?;
    loop {
        let path = parent.join(format!(
            "{prefix}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot reserve staging directory: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replaces_existing_and_cleans_temporary_on_failure() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state");
        write_atomic(&p, b"old").unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new");
        let dir = d.path().join("directory");
        fs::create_dir(&dir).unwrap();
        assert!(write_atomic(&dir, b"bad").is_err());
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 2);
    }
    #[test]
    fn stale_snapshot_is_not_published_and_staging_is_unique() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("lua");
        fs::write(&p, b"current").unwrap();
        assert!(write_if_unchanged(&p, b"old", b"replacement").is_err());
        assert_eq!(fs::read(&p).unwrap(), b"current");
        assert_ne!(
            create_staging(d.path(), "test").unwrap(),
            create_staging(d.path(), "test").unwrap()
        );
    }
}
