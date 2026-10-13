//! Crack file IO: atomic publication, staging collection and empty-parent
//! cleanup. Never leaves a partial file behind a failed copy.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::hashing::hash_file;

pub(crate) fn atomic_copy(
    source: &Path,
    destination: &Path,
    expected: &str,
    replace: bool,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or("Crack destination has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".aether-crack-{:016x}.tmp", rand::random::<u64>()));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| format!("Stage {}: {e}", destination.display()))?;
    let result = (|| {
        let permissions = fs::metadata(source)
            .map_err(|e| e.to_string())?
            .permissions();
        let mut input =
            fs::File::open(source).map_err(|e| format!("Read {}: {e}", source.display()))?;
        io::copy(&mut input, &mut output).map_err(|e| format!("Copy {}: {e}", source.display()))?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(input);
        drop(output);
        if hash_file(&temporary)? != expected {
            return Err(format!(
                "Crack source changed during copy: {}",
                source.display()
            ));
        }
        if replace {
            fs::rename(&temporary, destination)
                .map_err(|e| format!("Publish {}: {e}", destination.display()))?;
        } else {
            // Same-directory hard-link publication cannot overwrite an original.
            // Unsupported backup volumes fail BEFORE touching live game files.
            fs::hard_link(&temporary, destination).map_err(|e| {
                format!("Publish immutable original {}: {e}", destination.display())
            })?;
        }
        fs::remove_file(&temporary)
            .or_else(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|e| e.to_string())?;
        fs::set_permissions(destination, permissions).map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// Collect every regular staged file, including documents and other payload.
pub(crate) fn collect_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    collect_files_into(root, &mut result)?;
    result.sort();
    Ok(result)
}

fn collect_files_into(directory: &Path, result: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            collect_files_into(&entry.path(), result)?;
        } else if kind.is_file() {
            result.push(entry.path());
        } else {
            return Err(format!(
                "Unsupported staged crack entry {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn remove_empty_parents(file: &Path, stop_at: &Path) {
    let mut current = file.parent();
    while let Some(directory) = current {
        if directory == stop_at {
            break;
        }
        if fs::remove_dir(directory).is_err() {
            break;
        }
        current = directory.parent();
    }
}
