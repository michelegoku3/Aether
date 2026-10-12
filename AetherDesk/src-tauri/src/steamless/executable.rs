use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::external_tools::constants::{is_steamless_backup_name, is_steamless_unpacked_name};

pub fn validate_executable(exe_path: &Path, game_root: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(exe_path).map_err(|e| {
        format!(
            "Selected executable was not found at {}: {e}",
            exe_path.display()
        )
    })?;
    let linked = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        linked || metadata.file_attributes() & 0x400 != 0
    };
    if linked || !metadata.is_file() {
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

pub fn remove_stale_unpacked_outputs(exe_path: &Path) -> Result<(), String> {
    for candidate in unpacked_output_candidates(exe_path) {
        match fs::remove_file(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Cannot remove stale Steamless output {}: {error}; original unchanged",
                    candidate.display()
                ))
            }
        }
    }
    Ok(())
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

/// Checks a bounded PE header/section table and every raw section range.
/// This is structural plausibility, not a signature or executable-health check.
pub(crate) fn plausible_pe_machine(file: &mut fs::File) -> Result<u16, String> {
    fn read_at(file: &mut fs::File, offset: u64, bytes: &mut [u8]) -> Result<(), String> {
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        file.read_exact(bytes)
            .map_err(|e| format!("Truncated PE header: {e}"))
    }
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let mut dos = [0_u8; 64];
    read_at(file, 0, &mut dos)?;
    if &dos[..2] != b"MZ" {
        return Err("Missing DOS signature".into());
    }
    let pe = u64::from(u32::from_le_bytes(
        dos[60..64].try_into().map_err(|e| format!("{e}"))?,
    ));
    if pe < 64 || pe + 24 > length {
        return Err("Invalid PE offset".into());
    }
    let mut coff = [0_u8; 24];
    read_at(file, pe, &mut coff)?;
    if &coff[..4] != b"PE\0\0" {
        return Err("Missing PE signature".into());
    }
    let machine = u16::from_le_bytes([coff[4], coff[5]]);
    let sections = u16::from_le_bytes([coff[6], coff[7]]);
    let optional_size = u16::from_le_bytes([coff[20], coff[21]]);
    let characteristics = u16::from_le_bytes([coff[22], coff[23]]);
    let (magic, base) = match machine {
        0x014c => (0x010b_u16, 96_usize),
        0x8664 => (0x020b_u16, 112_usize),
        _ => return Err("Unsupported PE architecture (expected x86 or x64)".into()),
    };
    if sections == 0
        || sections > 96
        || usize::from(optional_size) < base
        || characteristics & 2 == 0
        || characteristics & 0x2000 != 0
    {
        return Err("Invalid executable COFF header".into());
    }
    let table = pe + 24 + u64::from(optional_size);
    let table_end = table + u64::from(sections) * 40;
    if table_end > length {
        return Err("Truncated PE section table".into());
    }
    let mut optional = vec![0_u8; usize::from(optional_size)];
    read_at(file, pe + 24, &mut optional)?;
    if u16::from_le_bytes([optional[0], optional[1]]) != magic {
        return Err("PE optional header does not match the architecture".into());
    }
    let field = |offset: usize| -> Result<u32, String> {
        let bytes: [u8; 4] = optional[offset..offset + 4]
            .try_into()
            .map_err(|e| format!("{e}"))?;
        Ok(u32::from_le_bytes(bytes))
    };
    let section_alignment = field(32)?;
    let file_alignment = field(36)?;
    let image_size = u64::from(field(56)?);
    let headers_size = u64::from(field(60)?);
    let directory_count = usize::try_from(field(base - 4)?).map_err(|e| e.to_string())?;
    if !section_alignment.is_power_of_two()
        || !file_alignment.is_power_of_two()
        || section_alignment < file_alignment
        || headers_size < table_end
        || headers_size > length
        || image_size < headers_size
        || directory_count > (optional.len() - base) / 8
    {
        return Err("Inconsistent PE optional header".into());
    }
    let mut has_payload = false;
    for index in 0..sections {
        let mut header = [0_u8; 40];
        read_at(file, table + u64::from(index) * 40, &mut header)?;
        let value = |offset: usize| -> Result<u64, String> {
            let bytes: [u8; 4] = header[offset..offset + 4]
                .try_into()
                .map_err(|e| format!("{e}"))?;
            Ok(u64::from(u32::from_le_bytes(bytes)))
        };
        let virtual_size = value(8)?;
        let virtual_address = value(12)?;
        let raw_size = value(16)?;
        let raw_pointer = value(20)?;
        if virtual_address + virtual_size.max(raw_size) > image_size {
            return Err("PE section is outside the declared image".into());
        }
        if raw_size != 0 {
            if raw_pointer < headers_size || raw_pointer + raw_size > length {
                return Err("Truncated PE section payload".into());
            }
            has_payload = true;
        }
    }
    if !has_payload {
        return Err("PE executable has no section payload".into());
    }
    Ok(machine)
}
