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
use std::path::{Component, Path, PathBuf};

/// Default password tried for protected archives (passed down from the caller).
pub fn stage_source(source: &Path, staging: &Path, default_password: &str) -> Result<(), String> {
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        // A `.zip` that is actually 7z/RAR-packed still gets a chance, but
        // every attempt reports its own cause: a wrong password used to
        // surface as "not a valid archive", hiding the real problem.
        "zip" => extract_zip(source, staging, default_password).or_else(|zip_err| {
            extract_7z(source, staging, default_password).or_else(|sevenz_err| {
                extract_rar(source, staging, default_password).map_err(|rar_err| {
                    format!(
                        "Failed to extract archive {}: not a valid ZIP ({zip_err}), 7Z ({sevenz_err}), or RAR archive ({rar_err})",
                        source.display()
                    )
                })
            })
        }),
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
    crate::core::state_io::create_staging(&LocalAppPaths::temp_dir(), &format!("crack_{app_id}"))
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
            fs::remove_dir_all(&path).map_err(|error| {
                format!("Failed to clear staging dir {}: {}", path.display(), error)
            })?;
        } else {
            fs::remove_file(&path).map_err(|error| {
                format!("Failed to clear staging file {}: {}", path.display(), error)
            })?;
        }
    }
    Ok(())
}

/// Remove the staging directory entirely.
pub fn remove_staging(staging: &Path) -> Result<(), String> {
    if staging.exists() {
        fs::remove_dir_all(staging).map_err(|error| {
            format!("Failed to remove staging {}: {}", staging.display(), error)
        })?;
    }
    Ok(())
}

/// Copy a loose (non-archive) crack file into the staging root.
fn copy_loose_file(source: &Path, staging: &Path) -> Result<(), String> {
    let name = source
        .file_name()
        .ok_or_else(|| format!("Crack file has no valid name: {}", source.display()))?;
    let dest = staging.join(name);
    fs::copy(source, &dest)
        .map_err(|error| format!("Failed to stage crack file {}: {}", dest.display(), error))?;
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
        // One decrypt-aware entry provides both metadata and content. The
        // passwordless reader rejects encrypted entries before exposing a name.
        let mut entry = archive
            .by_index_decrypt(index, default_password.as_bytes())
            .map_err(|error| {
                format!(
                    "Failed to open ZIP entry {index} in {} (default password tried): {error}",
                    source.display()
                )
            })?;
        let Some(relative) = sanitize_rel_path(entry.name()) else {
            continue; // skip unsafe (absolute / `..`) or empty paths
        };
        let out_path = dest.join(&relative);

        if entry.is_dir() {
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

        // Read through EOF/CRC into private staging, never publish a partial
        // file if decryption, decompression, checksum or writing fails.
        let parent = out_path.parent().ok_or("ZIP destination has no parent")?;
        let temporary = parent.join(format!(".aether-zip-{:016x}.tmp", rand::random::<u64>()));
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("Stage ZIP entry: {e}"))?;
        let copied = std::io::copy(&mut entry, &mut output)
            .map_err(|e| format!("Read/write ZIP entry {index} in {}: {e}", source.display()));
        drop(output);
        let result = copied.and_then(|_| {
            fs::rename(&temporary, &out_path)
                .map_err(|e| format!("Publish extracted file {}: {e}", out_path.display()))
        });
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
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
            format!(
                "Failed to write extracted file {}: {}",
                out_path.display(),
                error
            )
        })?;

        archive = rest;
    }

    Ok(())
}

/// Wrap a RAR error into a readable message.
/// The tried password is deliberately NOT included: error strings reach the
/// UI and the session log, and the default password is often reused.
fn rar_password_hint(
    source: &Path,
    _default_password: &str,
    error: impl std::fmt::Display,
) -> String {
    format!(
        "Failed to extract RAR {} (stored password tried): {}",
        source.display(),
        error
    )
}

/// Extract a `.7z` archive into `dest` using the pure-Rust `sevenz-rust2` crate.
///
/// Tries an unencrypted pass first, then retries with `default_password` for
/// protected archives. Every entry goes through the same `sanitize_rel_path`
/// gate as ZIP/RAR (absolute / `..` paths are skipped), and files are
/// published atomically (temp + rename) like the ZIP path — never through
/// the library's direct-to-`dest` extraction, which performs no validation.
/// A clear error is reported if both passes fail (never including the password).
fn extract_7z(source: &Path, dest: &Path, default_password: &str) -> Result<(), String> {
    match extract_7z_with(source, dest, None) {
        Ok(()) => Ok(()),
        Err(plain_err) => extract_7z_with(source, dest, Some(default_password)).map_err(|error| {
            format!(
                "Failed to extract 7z {} (tried plain [{plain_err}] and stored password [{error}])",
                source.display(),
            )
        }),
    }
}

fn extract_7z_with(source: &Path, dest: &Path, password: Option<&str>) -> Result<(), String> {
    use sevenz_rust2::{ArchiveReader, Password};

    let credential = password.map_or_else(Password::empty, Password::from);
    let mut reader = ArchiveReader::open(source, credential)
        .map_err(|error| format!("Failed to open 7z {}: {}", source.display(), error))?;

    reader
        .for_each_entries(|entry, input| {
            if entry.is_directory() {
                if let Some(relative) = sanitize_rel_path(entry.name()) {
                    fs::create_dir_all(dest.join(&relative)).map_err(io_other)?;
                }
                return Ok(true);
            }
            let Some(relative) = sanitize_rel_path(entry.name()) else {
                return Ok(true); // skip unsafe (absolute / `..`) like ZIP/RAR
            };
            let out_path = dest.join(&relative);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(io_other)?;
            }
            // Atomic publish like the ZIP path: never leave a partial file if
            // decompression or the write fails mid-entry.
            let parent = out_path
                .parent()
                .ok_or_else(|| io_other("7z destination has no parent"))?;
            let temporary = parent.join(format!(".aether-7z-{:016x}.tmp", rand::random::<u64>()));
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(io_other)?;
            let copied = std::io::copy(input, &mut output).map_err(io_other);
            drop(output);
            match copied {
                Ok(_) => fs::rename(&temporary, &out_path).map_err(io_other)?,
                Err(error) => {
                    let _ = fs::remove_file(&temporary);
                    return Err(error.into());
                }
            }
            Ok(true)
        })
        .map_err(|error| format!("Failed to extract 7z {}: {}", source.display(), error))
}

/// Wrap any displayable error into the `std::io::Error` that `for_each_entries`
/// converts into a 7z error via its `From<std::io::Error>` impl.
fn io_other(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    // Genuine stored ZipCrypto ZIPs: fixed encryption header, password online-fix.me.
    // Generated independently and decrypted/CRC-checked with Python zipfile.
    const PLAIN_ZIP: &[u8] = &[
        0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00,
        0x6e, 0x65, 0x73, 0x74, 0x65, 0x64, 0x2f, 0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x08,
        0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0xc6, 0xf0, 0xe7, 0x96, 0x0d, 0x00, 0x00, 0x00, 0x0d,
        0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x6e, 0x65, 0x73, 0x74, 0x65, 0x64, 0x2f, 0x70,
        0x6c, 0x61, 0x69, 0x6e, 0x2e, 0x74, 0x78, 0x74, 0x70, 0x6c, 0x61, 0x69, 0x6e, 0x20, 0x70,
        0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x03, 0x14, 0x00, 0x00,
        0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
        0x00, 0xed, 0x41, 0x00, 0x00, 0x00, 0x00, 0x6e, 0x65, 0x73, 0x74, 0x65, 0x64, 0x2f, 0x50,
        0x4b, 0x01, 0x02, 0x14, 0x03, 0x14, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00,
        0xc6, 0xf0, 0xe7, 0x96, 0x0d, 0x00, 0x00, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x81, 0x25, 0x00, 0x00, 0x00,
        0x6e, 0x65, 0x73, 0x74, 0x65, 0x64, 0x2f, 0x70, 0x6c, 0x61, 0x69, 0x6e, 0x2e, 0x74, 0x78,
        0x74, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x73, 0x00,
        0x00, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    const ENCRYPTED_ZIP: &[u8] = &[
        0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x29,
        0x77, 0x82, 0x48, 0x1d, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64, 0x2e, 0x64, 0x6c, 0x6c, 0x37, 0xc5, 0xeb, 0xbc,
        0xae, 0x93, 0x9a, 0x5c, 0xcc, 0x8f, 0xca, 0x14, 0x9a, 0x02, 0xdf, 0x76, 0x62, 0x9a, 0xd1,
        0xdd, 0xc5, 0xb3, 0x30, 0x42, 0xba, 0x69, 0xb6, 0x1d, 0xd9, 0x50, 0x4b, 0x01, 0x02, 0x14,
        0x03, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x29, 0x77, 0x82, 0x48,
        0x1d, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x81, 0x00, 0x00, 0x00, 0x00, 0x70, 0x61, 0x79, 0x6c,
        0x6f, 0x61, 0x64, 0x2e, 0x64, 0x6c, 0x6c, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x39, 0x00, 0x00, 0x00, 0x46, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    const MIXED_ZIP: &[u8] = &[
        0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0xc6,
        0xf0, 0xe7, 0x96, 0x0d, 0x00, 0x00, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00,
        0x70, 0x6c, 0x61, 0x69, 0x6e, 0x2e, 0x74, 0x78, 0x74, 0x70, 0x6c, 0x61, 0x69, 0x6e, 0x20,
        0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64, 0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x01, 0x08,
        0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x29, 0x77, 0x82, 0x48, 0x1d, 0x00, 0x00, 0x00, 0x11,
        0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64, 0x2e,
        0x64, 0x6c, 0x6c, 0x37, 0xc5, 0xeb, 0xbc, 0xae, 0x93, 0x9a, 0x5c, 0xcc, 0x8f, 0xca, 0x14,
        0x9a, 0x02, 0xdf, 0x76, 0x62, 0x9a, 0xd1, 0xdd, 0xc5, 0xb3, 0x30, 0x42, 0xba, 0x69, 0xb6,
        0x1d, 0xd9, 0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00,
        0x00, 0x00, 0x6e, 0x65, 0x73, 0x74, 0x65, 0x64, 0x2f, 0x37, 0xc5, 0xeb, 0xbc, 0xae, 0x93,
        0x9a, 0x5c, 0xcc, 0x8f, 0xca, 0x5c, 0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x01, 0x08, 0x00,
        0x00, 0x00, 0x00, 0x21, 0x00, 0x38, 0x76, 0x19, 0xce, 0x17, 0x00, 0x00, 0x00, 0x0b, 0x00,
        0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0x6e, 0x65, 0x73, 0x74, 0x65, 0x64, 0x2f, 0x70, 0x61,
        0x79, 0x6c, 0x6f, 0x61, 0x64, 0x2e, 0x74, 0x78, 0x74, 0x37, 0xc5, 0xeb, 0xbc, 0xae, 0x93,
        0x9a, 0x5c, 0xcc, 0x8f, 0xca, 0x92, 0x71, 0x67, 0x04, 0x0d, 0x72, 0xf4, 0x81, 0x36, 0x9a,
        0x6e, 0xbc, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x03, 0x14, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00,
        0x00, 0x21, 0x00, 0xc6, 0xf0, 0xe7, 0x96, 0x0d, 0x00, 0x00, 0x00, 0x0d, 0x00, 0x00, 0x00,
        0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x81, 0x00,
        0x00, 0x00, 0x00, 0x70, 0x6c, 0x61, 0x69, 0x6e, 0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b, 0x01,
        0x02, 0x14, 0x03, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x29, 0x77,
        0x82, 0x48, 0x1d, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x81, 0x34, 0x00, 0x00, 0x00, 0x70, 0x61,
        0x79, 0x6c, 0x6f, 0x61, 0x64, 0x2e, 0x64, 0x6c, 0x6c, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x03,
        0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0c,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x10, 0x00, 0xed, 0x41, 0x7a, 0x00, 0x00, 0x00, 0x6e, 0x65, 0x73, 0x74, 0x65,
        0x64, 0x2f, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x03, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00,
        0x00, 0x21, 0x00, 0x38, 0x76, 0x19, 0xce, 0x17, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x81, 0xab,
        0x00, 0x00, 0x00, 0x6e, 0x65, 0x73, 0x74, 0x65, 0x64, 0x2f, 0x70, 0x61, 0x79, 0x6c, 0x6f,
        0x61, 0x64, 0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x04,
        0x00, 0x04, 0x00, 0xe5, 0x00, 0x00, 0x00, 0xf2, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    const UNSAFE_ZIP: &[u8] = &[
        0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x6d,
        0xe7, 0xa4, 0xb8, 0x1c, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x17, 0x00, 0x00, 0x00,
        0x2e, 0x2e, 0x2f, 0x6f, 0x75, 0x74, 0x73, 0x69, 0x64, 0x65, 0x2d, 0x73, 0x65, 0x6e, 0x74,
        0x69, 0x6e, 0x65, 0x6c, 0x2e, 0x74, 0x78, 0x74, 0x37, 0xc5, 0xeb, 0xbc, 0xae, 0x93, 0x9a,
        0x5c, 0xcc, 0x8f, 0xca, 0xe4, 0x79, 0x3f, 0xe0, 0x2b, 0xd2, 0x8b, 0xb1, 0x4e, 0x38, 0xef,
        0x0c, 0xd9, 0x0b, 0x5c, 0xdd, 0x5f, 0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x01, 0x08, 0x00,
        0x00, 0x00, 0x00, 0x21, 0x00, 0x8a, 0x51, 0x39, 0xa0, 0x1a, 0x00, 0x00, 0x00, 0x0e, 0x00,
        0x00, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x2f, 0x61, 0x62, 0x73, 0x6f, 0x6c, 0x75, 0x74, 0x65,
        0x2e, 0x74, 0x78, 0x74, 0x37, 0xc5, 0xeb, 0xbc, 0xae, 0x93, 0x9a, 0x5c, 0xcc, 0x8f, 0xca,
        0xfc, 0xb8, 0x5c, 0xbf, 0x40, 0x64, 0x67, 0xd8, 0x68, 0xab, 0xe3, 0x14, 0xa0, 0xe3, 0xfd,
        0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x72,
        0xbb, 0x60, 0x55, 0x18, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x73, 0x61, 0x66, 0x65, 0x2f, 0x6f, 0x6b, 0x2e, 0x74, 0x78, 0x74, 0x37, 0xc5, 0xeb, 0xbc,
        0xae, 0x93, 0x9a, 0x5c, 0xcc, 0x8f, 0xca, 0x09, 0x6e, 0x16, 0xb7, 0xf5, 0xdd, 0x05, 0x28,
        0xcd, 0xe0, 0x41, 0x84, 0x52, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x03, 0x14, 0x00, 0x01, 0x08,
        0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x6d, 0xe7, 0xa4, 0xb8, 0x1c, 0x00, 0x00, 0x00, 0x10,
        0x00, 0x00, 0x00, 0x17, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xa4, 0x81, 0x00, 0x00, 0x00, 0x00, 0x2e, 0x2e, 0x2f, 0x6f, 0x75, 0x74, 0x73, 0x69, 0x64,
        0x65, 0x2d, 0x73, 0x65, 0x6e, 0x74, 0x69, 0x6e, 0x65, 0x6c, 0x2e, 0x74, 0x78, 0x74, 0x50,
        0x4b, 0x01, 0x02, 0x14, 0x03, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00,
        0x8a, 0x51, 0x39, 0xa0, 0x1a, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00, 0x0d, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x81, 0x51, 0x00, 0x00, 0x00,
        0x2f, 0x61, 0x62, 0x73, 0x6f, 0x6c, 0x75, 0x74, 0x65, 0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b,
        0x01, 0x02, 0x14, 0x03, 0x14, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0x72,
        0xbb, 0x60, 0x55, 0x18, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x81, 0x96, 0x00, 0x00, 0x00, 0x73,
        0x61, 0x66, 0x65, 0x2f, 0x6f, 0x6b, 0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b, 0x05, 0x06, 0x00,
        0x00, 0x00, 0x00, 0x03, 0x00, 0x03, 0x00, 0xb9, 0x00, 0x00, 0x00, 0xd7, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];

    fn fixture(bytes: &[u8]) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("fixture.zip");
        let staging = temp.path().join("stage");
        fs::create_dir_all(&staging).unwrap();
        fs::write(&source, bytes).unwrap();
        fs::write(temp.path().join("outside-sentinel.txt"), b"protected").unwrap();
        (temp, source, staging)
    }

    #[test]
    fn genuinely_encrypted_zip_extracts_with_the_correct_password() {
        let (_temp, source, staging) = fixture(ENCRYPTED_ZIP);
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(ENCRYPTED_ZIP)).unwrap();
        assert!(
            archive.by_index(0).is_err(),
            "fixture must require the decrypt-aware reader"
        );
        extract_zip(&source, &staging, "online-fix.me").unwrap();
        assert_eq!(
            fs::read(staging.join("payload.dll")).unwrap(),
            b"encrypted payload"
        );
    }

    #[test]
    fn wrong_password_is_rejected_without_publishing_a_file() {
        let (_temp, source, staging) = fixture(ENCRYPTED_ZIP);
        assert!(extract_zip(&source, &staging, "wrong-password").is_err());
        assert!(!staging.join("payload.dll").exists());
    }

    #[test]
    fn mixed_encrypted_plain_and_directory_entries_use_the_same_reader() {
        let (_temp, source, staging) = fixture(MIXED_ZIP);
        stage_source(&source, &staging, "online-fix.me").unwrap();
        assert_eq!(
            fs::read(staging.join("plain.txt")).unwrap(),
            b"plain payload"
        );
        assert_eq!(
            fs::read(staging.join("payload.dll")).unwrap(),
            b"encrypted payload"
        );
        assert_eq!(
            fs::read(staging.join("nested/payload.txt")).unwrap(),
            b"nested body"
        );
    }

    #[test]
    fn plain_zip_is_still_readable_when_a_password_is_supplied() {
        let (_temp, source, staging) = fixture(PLAIN_ZIP);
        extract_zip(&source, &staging, "unused-password").unwrap();
        assert_eq!(
            fs::read(staging.join("nested/plain.txt")).unwrap(),
            b"plain payload"
        );
    }

    #[test]
    fn encrypted_unsafe_names_are_skipped_and_outside_sentinel_is_unchanged() {
        let (temp, source, staging) = fixture(UNSAFE_ZIP);
        extract_zip(&source, &staging, "online-fix.me").unwrap();
        assert_eq!(
            fs::read(temp.path().join("outside-sentinel.txt")).unwrap(),
            b"protected"
        );
        assert!(!staging.join("absolute.txt").exists());
        assert_eq!(
            fs::read(staging.join("safe/ok.txt")).unwrap(),
            b"safe payload"
        );
    }

    #[test]
    fn corrupted_encrypted_payload_is_not_published_even_after_password_acceptance() {
        let mut damaged = ENCRYPTED_ZIP.to_vec();
        damaged[30 + "payload.dll".len() + 12] ^= 0xff;
        let (_temp, source, staging) = fixture(&damaged);
        assert!(extract_zip(&source, &staging, "online-fix.me").is_err());
        assert!(!staging.join("payload.dll").exists());
        assert!(
            fs::read_dir(staging).unwrap().next().is_none(),
            "private partial temp must be cleaned"
        );
    }

    #[test]
    fn encrypted_zip_pipeline_preserves_original_and_records_the_applied_target() {
        use crate::core::backup::GameBackup;
        let (temp, source, staging) = fixture(ENCRYPTED_ZIP);
        let game = temp.path().join("game");
        let backup_root = temp.path().join("backup");
        fs::create_dir_all(&game).unwrap();
        for directory in ["lua", "original", "crack"] {
            fs::create_dir_all(backup_root.join(directory)).unwrap();
        }
        fs::write(game.join("payload.dll"), b"original sentinel").unwrap();
        let backup = GameBackup::for_tests(backup_root);
        let report = crate::crack::apply_sources_in_staging(
            424244,
            &game,
            &backup,
            &[source.to_str().unwrap().to_string()],
            false,
            &staging,
        )
        .unwrap();
        assert_eq!(report.applied, 1);
        assert_eq!(
            fs::read(game.join("payload.dll")).unwrap(),
            b"encrypted payload"
        );
        assert_eq!(
            fs::read(backup.original_dir().join("payload.dll")).unwrap(),
            b"original sentinel"
        );
        assert_eq!(
            backup.read_crack_inventory(424244).unwrap(),
            vec!["payload.dll"]
        );
        assert!(!staging.exists());
    }

    #[test]
    fn zip_fallback_reports_every_attempt_cause() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("bogus.zip");
        fs::write(&source, b"definitely not an archive").unwrap();
        let staging = temp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let secret = "s3cr3t-pw-zzz";
        let err = stage_source(&source, &staging, secret).unwrap_err();
        for marker in ["ZIP (", "7Z (", "RAR ("] {
            assert!(err.contains(marker), "missing {marker} in: {err}");
        }
        assert!(!err.contains(secret), "password leaked: {err}");
    }

    #[test]
    fn archive_error_messages_never_contain_the_password() {
        let secret = "s3cr3t-reused-password";
        let missing = std::path::Path::new("definitely-missing-archive.bin");
        let rar_err = rar_password_hint(missing, secret, "boom");
        assert!(
            !rar_err.contains(secret),
            "RAR error leaked password: {rar_err}"
        );
        let dest = tempfile::tempdir().unwrap();
        let err = extract_7z(missing, dest.path(), secret).unwrap_err();
        assert!(!err.contains(secret), "7z error leaked password: {err}");
    }

    #[test]
    fn sevenz_traversal_entries_never_escape_staging() {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("evil.7z");
        let staging = temp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let sentinel = temp.path().join("outside-sentinel.txt");
        fs::write(&sentinel, b"protected").unwrap();
        {
            let mut writer = ArchiveWriter::create(&source).unwrap();
            let mut safe: &[u8] = b"safe payload";
            writer
                .push_archive_entry(ArchiveEntry::new_file("safe/ok.txt"), Some(&mut safe))
                .unwrap();
            let mut evil: &[u8] = b"evil payload";
            writer
                .push_archive_entry(ArchiveEntry::new_file("../outside.txt"), Some(&mut evil))
                .unwrap();
            writer.finish().unwrap();
        }
        extract_7z(&source, &staging, "").unwrap();
        assert_eq!(
            fs::read(staging.join("safe/ok.txt")).unwrap(),
            b"safe payload"
        );
        assert!(!staging.join("outside.txt").exists());
        assert!(!temp.path().join("outside.txt").exists());
        assert_eq!(fs::read(&sentinel).unwrap(), b"protected");
    }
}
