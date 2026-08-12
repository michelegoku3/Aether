// Archive staging & extraction.
//
// Handles turning one crack "source" into staged content ready for
// `locate`/`apply`:
//   * `.zip` → extracted with the `zip` crate (password-aware)
//   * `.rar` → extracted with the `unrar` crate (password-aware)
//   * `.7z`  → extracted with the `sevenz-rust2` crate (password-aware)
//   * anything else → treated as a loose crack file, copied into the staging
//     root as-is
//
// All content is staged under a temporary directory that callers are expected
// to clean up. Extraction paths are sanitized to prevent path traversal
// ("zip-slip").
use crate::core::paths::LocalAppPaths;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Default password tried for protected archives (passed down from the caller).
pub fn stage_source(source: &Path, staging: &Path, default_password: &str) -> Result<(), String> {
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        "zip" => {
            extract_zip(source, staging, default_password)
                .or_else(|_| extract_7z(source, staging, default_password))
                .or_else(|_| extract_rar(source, staging, default_password))
                .map_err(|_| {
                    format!(
                        "Failed to extract archive {}: not a valid ZIP, 7Z, or RAR archive",
                        source.display()
                    )
                })
        }
        "rar" => extract_rar(source, staging, default_password),
        "7z" => extract_7z(source, staging, default_password),
        _ => copy_loose_file(source, staging),
    }
}

/// Create a unique staging directory for an app's crack run.
///
/// The staging lives under `AetherData/tmp` (not the OS temp folder) so that,
/// once the user adds AetherData to Windows Defender exclusions, the extracted
/// crack files are not flagged by the antivirus.
pub fn create_staging(app_id: u32) -> Result<PathBuf, String> {
    let staging = LocalAppPaths::temp_dir().join(format!(
        "crack_{}_{}",
        app_id,
        std::process::id()
    ));
    fs::create_dir_all(&staging)
        .map_err(|error| format!("Failed to create staging folder {}: {}", staging.display(), error))?;
    Ok(staging)
}

/// Remove every file/folder currently inside `staging` (for the next source).
pub fn clear_staging_contents(staging: &Path) -> Result<(), String> {
    if !staging.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(staging)
        .map_err(|error| format!("Failed to read staging {}: {}", staging.display(), error))?
    {
        let entry = entry.map_err(|error| format!("Failed to read staging entry: {}", error))?;
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|error| format!("Failed to clear staging dir {}: {}", path.display(), error))?;
        } else {
            fs::remove_file(&path)
                .map_err(|error| format!("Failed to clear staging file {}: {}", path.display(), error))?;
        }
    }
    Ok(())
}

/// Remove the staging directory entirely.
pub fn remove_staging(staging: &Path) -> Result<(), String> {
    if staging.exists() {
        fs::remove_dir_all(staging)
            .map_err(|error| format!("Failed to remove staging {}: {}", staging.display(), error))?;
    }
    Ok(())
}

/// Copy a loose (non-archive) crack file into the staging root.
fn copy_loose_file(source: &Path, staging: &Path) -> Result<(), String> {
    let name = source
        .file_name()
        .ok_or_else(|| format!("Crack file has no valid name: {}", source.display()))?;
    let dest = staging.join(name);
    fs::copy(source, &dest).map_err(|error| {
        format!("Failed to stage crack file {}: {}", dest.display(), error)
    })?;
    Ok(())
}

/// Extract a `.zip` archive into `dest`. Tries `default_password` for encrypted
/// entries and reports a clear error if the password is rejected.
fn extract_zip(source: &Path, dest: &Path, default_password: &str) -> Result<(), String> {
    let file = fs::File::open(source)
        .map_err(|error| format!("Failed to open ZIP {}: {}", source.display(), error))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("Failed to read ZIP {}: {}", source.display(), error))?;

    for index in 0..archive.len() {
        // Inspect the entry first (name, whether it is a directory).
        let (name, is_dir) = {
            let entry = archive
                .by_index(index)
                .map_err(|error| format!("Failed to read ZIP entry {}: {}", index, error))?;
            (entry.name().to_string(), entry.is_dir())
        };

        let Some(relative) = sanitize_rel_path(&name) else {
            continue; // skip unsafe (absolute / `..`) or empty paths
        };
        let out_path = dest.join(&relative);

        if is_dir {
            fs::create_dir_all(&out_path).map_err(|error| {
                format!("Failed to create folder {}: {}", out_path.display(), error)
            })?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create folder {}: {}", parent.display(), error)
            })?;
        }

        // Use the decrypt-aware reader: it transparently reads both plain and
        // encrypted entries and reports a clear error if the password is wrong.
        let mut entry = archive
            .by_index_decrypt(index, default_password.as_bytes())
            .map_err(|_| {
                format!(
                    "Failed to read encrypted entry {} in {} (tried default password '{}').",
                    index,
                    source.display(),
                    default_password
                )
            })?;

        let mut buffer = Vec::new();
        entry.read_to_end(&mut buffer).map_err(|error| {
            format!("Failed to read entry in {}: {}", source.display(), error)
        })?;

        fs::write(&out_path, &buffer).map_err(|error| {
            format!("Failed to write extracted file {}: {}", out_path.display(), error)
        })?;
    }

    Ok(())
}

/// Extract a `.rar` archive into `dest` using the `unrar` crate with
/// `default_password`. Reports a clear error if the password is rejected.
///
/// The `unrar` crate drives archives through a typestate cursor: we read each
/// header, and for every file entry we read its bytes and write them ourselves
/// (applying the same path sanitization as ZIP) to prevent path traversal.
fn extract_rar(source: &Path, dest: &Path, default_password: &str) -> Result<(), String> {
    use unrar::Archive;

    let mut archive = Archive::with_password(source, default_password)
        .open_for_processing()
        .map_err(|error| rar_password_hint(source, default_password, error))?;

    loop {
        // `read_header()` returns None once the archive is exhausted.
        let Some(opened) = archive
            .read_header()
            .map_err(|error| rar_password_hint(source, default_password, error))?
        else {
            break;
        };

        // `opened` is in "before file" state; `entry()` gives its metadata.
        if opened.entry().is_directory() {
            archive = opened
                .skip()
                .map_err(|error| rar_password_hint(source, default_password, error))?;
            continue;
        }

        let filename = opened.entry().filename.to_string_lossy().to_string();
        let Some(relative) = sanitize_rel_path(&filename) else {
            archive = opened
                .skip()
                .map_err(|error| rar_password_hint(source, default_password, error))?;
            continue;
        };

        let (data, rest) = opened
            .read()
            .map_err(|error| rar_password_hint(source, default_password, error))?;

        let out_path = dest.join(&relative);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create folder {}: {}", parent.display(), error)
            })?;
        }
        fs::write(&out_path, &data).map_err(|error| {
            format!("Failed to write extracted file {}: {}", out_path.display(), error)
        })?;

        archive = rest;
    }

    Ok(())
}

/// Wrap a RAR error into a readable message that hints at the tried password.
fn rar_password_hint(
    source: &Path,
    default_password: &str,
    error: impl std::fmt::Display,
) -> String {
    format!(
        "Failed to extract RAR {} (password '{}' tried): {}",
        source.display(),
        default_password,
        error
    )
}

/// Extract a `.7z` archive into `dest` using the pure-Rust `sevenz-rust2` crate.
///
/// Tries an unencrypted pass first, then retries with `default_password` for
/// protected archives. A clear error is reported if both fail.
fn extract_7z(source: &Path, dest: &Path, default_password: &str) -> Result<(), String> {
    let plain = sevenz_rust2::decompress_file(source, dest);
    if plain.is_ok() {
        return Ok(());
    }

    sevenz_rust2::decompress_file_with_password(source, dest, default_password.into())
        .map_err(|error| {
            format!(
                "Failed to extract 7z {} (tried plain and password '{}'): {}",
                source.display(),
                default_password,
                error
            )
        })
}

/// Normalize an archive entry name into a safe relative path, or `None` if it
/// must be skipped (absolute, contains `..`, or is empty/whitespace).
fn sanitize_rel_path(entry_name: &str) -> Option<String> {
    let normalized = entry_name.replace('\\', "/");
    let path = Path::new(&normalized);
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => continue,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let value = safe.to_string_lossy().to_string();
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}
