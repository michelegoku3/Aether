use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::external_tools::constants::{
    is_steamless_backup_name, is_steamless_unpacked_name,
};

pub fn validate_executable(exe_path: &Path, game_root: &Path) -> Result<(), String> {
    if !exe_path.is_file() {
        return Err(format!(
            "Selected executable was not found: {}",
            exe_path.display()
        ));
    }

    if !has_exe_extension(exe_path) {
        return Err("Steamless can only process Windows .exe files.".to_string());
    }

    let name_lower = exe_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();

    if is_steamless_unpacked_name(&name_lower) || is_steamless_backup_name(&name_lower) {
        return Err(
            "Select the original game executable, not a Steamless output or backup file."
                .to_string(),
        );
    }

    if !is_inside_game_root(exe_path, game_root)? {
        return Err(
            "Selected executable must be inside the selected game's install folder.".to_string(),
        );
    }

    if !has_mz_header(exe_path)? {
        return Err(format!(
            "{} is not a valid Windows PE executable (missing MZ header).",
            exe_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Selected file")
        ));
    }

    Ok(())
}

pub fn unpacked_output_candidates(exe_path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(file_name) = exe_path.file_name().and_then(|name| name.to_str()) {
        candidates.push(exe_path.with_file_name(format!("{}.unpacked.exe", file_name)));
    }

    if let Some(stem) = exe_path.file_stem().and_then(|stem| stem.to_str()) {
        let stem_candidate = exe_path.with_file_name(format!("{}.unpacked.exe", stem));
        if !candidates.iter().any(|path| path == &stem_candidate) {
            candidates.push(stem_candidate);
        }
    }

    candidates
}

pub fn unique_backup_path(exe_path: &Path) -> PathBuf {
    let file_name = exe_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("game.exe");
    let base = exe_path.with_file_name(format!("{}.steamstub.bak", file_name));
    if !base.exists() {
        return base;
    }

    for index in 1..=999 {
        let candidate = exe_path.with_file_name(format!("{}.steamstub.{}.bak", file_name, index));
        if !candidate.exists() {
            return candidate;
        }
    }

    exe_path.with_file_name(format!("{}.steamstub.latest.bak", file_name))
}

pub fn remove_stale_unpacked_outputs(exe_path: &Path) {
    for candidate in unpacked_output_candidates(exe_path) {
        if candidate.exists() {
            let _ = fs::remove_file(candidate);
        }
    }
}

fn has_exe_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

fn has_mz_header(path: &Path) -> Result<bool, String> {
    let mut file = fs::File::open(path)
        .map_err(|e| format!("Failed to open executable for validation: {}", e))?;
    let mut header = [0u8; 2];
    file.read_exact(&mut header)
        .map_err(|e| format!("Failed to read executable header: {}", e))?;
    Ok(&header == b"MZ")
}

fn is_inside_game_root(path: &Path, game_root: &Path) -> Result<bool, String> {
    let canonical_path = path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve selected executable path: {}", e))?;
    let canonical_root = game_root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve game install folder: {}", e))?;

    Ok(canonical_path.starts_with(canonical_root))
}
