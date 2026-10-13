//! Crack hashing helpers: content addressing for originals, payloads and
//! staged files. Backs the recovery inventory (`recovery`) and the atomic
//! publisher (`io`).
use std::fs;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

pub(crate) fn optional_hash(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => hash_file(path).map(Some),
        Ok(_) => Err(format!(
            "Expected a regular crack file at {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Inspect {}: {error}", path.display())),
    }
}

pub(crate) fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("Read {}: {e}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
