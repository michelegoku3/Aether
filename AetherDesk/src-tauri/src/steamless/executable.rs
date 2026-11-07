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

/// Page size of the x86/x64 images this parser accepts: the loader maps at
/// least one page for the PE headers.
const HEADER_MAPPING_PAGE: u64 = 0x1000;

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
    // The loader rounds SizeOfHeaders up to a page and reads the section table
    // from that mapping, so a declared header region shorter than the table end
    // still loads and must not be rejected here.
    let mapped_headers =
        headers_size.saturating_add(HEADER_MAPPING_PAGE - 1) & !(HEADER_MAPPING_PAGE - 1);
    let mapped_headers = mapped_headers.max(HEADER_MAPPING_PAGE);
    if !section_alignment.is_power_of_two()
        || !file_alignment.is_power_of_two()
        || section_alignment < file_alignment
        || headers_size == 0
        || table_end > mapped_headers
        || headers_size > length
        || image_size < headers_size
        || directory_count > (optional.len() - base) / 8
    {
        return Err(format!(
            "Inconsistent PE optional header: SectionAlignment=0x{section_alignment:X}, FileAlignment=0x{file_alignment:X}, SizeOfImage=0x{image_size:X}, SizeOfHeaders=0x{headers_size:X}, section table end=0x{table_end:X}, data directories={directory_count}"
        ));
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
        // Raw bytes pad to FileAlignment and may exceed the virtual size;
        // the loader maps VirtualSize (RawSize when VirtualSize is zero).
        let mapped_size = if virtual_size == 0 {
            raw_size
        } else {
            virtual_size
        };
        if virtual_address + mapped_size > image_size {
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

#[cfg(test)]
mod tests {
    use super::*;

    const E_LFANEW: usize = 0x80;
    const OPTIONAL: usize = E_LFANEW + 24;
    const OPTIONAL_SIZE: usize = 240;
    const SECTION: usize = OPTIONAL + OPTIONAL_SIZE;

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// One-section PE32+ sample mirroring the field layout under test.
    fn sample_pe() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x1000];
        bytes[..2].copy_from_slice(b"MZ");
        put_u32(&mut bytes, 60, u32::try_from(E_LFANEW).unwrap());
        bytes[E_LFANEW..E_LFANEW + 4].copy_from_slice(b"PE\0\0");
        put_u16(&mut bytes, E_LFANEW + 4, 0x8664);
        put_u16(&mut bytes, E_LFANEW + 6, 1);
        put_u16(
            &mut bytes,
            E_LFANEW + 20,
            u16::try_from(OPTIONAL_SIZE).unwrap(),
        );
        put_u16(&mut bytes, E_LFANEW + 22, 0x22);
        put_u16(&mut bytes, OPTIONAL, 0x020b);
        put_u32(&mut bytes, OPTIONAL + 32, 0x1000);
        put_u32(&mut bytes, OPTIONAL + 36, 0x200);
        put_u32(&mut bytes, OPTIONAL + 56, 0x2000);
        put_u32(&mut bytes, OPTIONAL + 60, 0x400);
        put_u32(&mut bytes, OPTIONAL + 108, 16);
        bytes[SECTION..SECTION + 5].copy_from_slice(b".text");
        put_u32(&mut bytes, SECTION + 8, 0x1000);
        put_u32(&mut bytes, SECTION + 12, 0x1000);
        put_u32(&mut bytes, SECTION + 16, 0x200);
        put_u32(&mut bytes, SECTION + 20, 0x400);
        bytes
    }

    fn check(bytes: &[u8]) -> Result<u16, String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.exe");
        fs::write(&path, bytes).unwrap();
        let mut file = fs::File::open(&path).unwrap();
        plausible_pe_machine(&mut file)
    }

    #[test]
    fn section_table_past_size_of_headers_but_inside_the_first_page_is_accepted() {
        // Eight section headers end at 0x2C8 while SizeOfHeaders is set to
        // 0x200: the loader still maps the first page for headers, so the
        // table fits even though the declared header region stops earlier.
        let mut bytes = sample_pe();
        put_u16(&mut bytes, E_LFANEW + 6, 8);
        put_u32(&mut bytes, OPTIONAL + 60, 0x200);
        assert_eq!(check(&bytes), Ok(0x8664));
    }

    #[test]
    fn zero_size_of_headers_is_rejected() {
        let mut bytes = sample_pe();
        put_u32(&mut bytes, OPTIONAL + 60, 0);
        let error = check(&bytes).unwrap_err();
        assert!(
            error.starts_with("Inconsistent PE optional header"),
            "{error}"
        );
    }

    #[test]
    fn raw_section_padding_beyond_the_image_is_accepted() {
        // SizeOfRawData pads to FileAlignment and may exceed the virtual size;
        // Windows maps VirtualSize, so raw padding must not fail containment.
        let mut bytes = sample_pe();
        put_u32(&mut bytes, SECTION + 8, 0x800);
        put_u32(&mut bytes, SECTION + 12, 0x1800);
        put_u32(&mut bytes, SECTION + 16, 0x1000);
        bytes.resize(0x1400, 0);
        assert_eq!(check(&bytes), Ok(0x8664));
    }

    #[test]
    fn section_virtual_extent_past_the_image_is_rejected() {
        let mut bytes = sample_pe();
        put_u32(&mut bytes, SECTION + 8, 0x1000);
        put_u32(&mut bytes, SECTION + 12, 0x1800);
        let error = check(&bytes).unwrap_err();
        assert!(error.contains("outside the declared image"), "{error}");
    }

    #[test]
    fn section_table_outside_the_mapped_header_region_is_rejected() {
        // e_lfanew moved to 0x2000: its section table ends at 0x2130, past the
        // page-rounded 0x2000 header mapping.
        let mut bytes = vec![0_u8; 0x3000];
        bytes[..2].copy_from_slice(b"MZ");
        put_u32(&mut bytes, 60, 0x2000);
        bytes[0x2000..0x2004].copy_from_slice(b"PE\0\0");
        put_u16(&mut bytes, 0x2004, 0x8664);
        put_u16(&mut bytes, 0x2006, 1);
        put_u16(&mut bytes, 0x2014, u16::try_from(OPTIONAL_SIZE).unwrap());
        put_u16(&mut bytes, 0x2016, 0x22);
        put_u16(&mut bytes, 0x2018, 0x020b);
        put_u32(&mut bytes, 0x2018 + 32, 0x1000);
        put_u32(&mut bytes, 0x2018 + 36, 0x200);
        put_u32(&mut bytes, 0x2018 + 56, 0x20000);
        put_u32(&mut bytes, 0x2018 + 60, 0x2000);
        put_u32(&mut bytes, 0x2018 + 108, 16);
        put_u32(&mut bytes, 0x2108 + 16, 0x200);
        put_u32(&mut bytes, 0x2108 + 20, 0x2200);
        let error = check(&bytes).unwrap_err();
        assert!(
            error.starts_with("Inconsistent PE optional header"),
            "{error}"
        );
    }

    #[test]
    fn inconsistent_header_error_names_the_offending_values() {
        // Diagnostics must identify the field: without values the failure can
        // only be guessed at from a user's log.
        let mut bytes = sample_pe();
        put_u32(&mut bytes, OPTIONAL + 60, 0x2000);
        let error = check(&bytes).unwrap_err();
        // sample_pe() holds one 40-byte section header at 0x188, so the table
        // ends at 0x1B0.
        assert!(error.contains("SizeOfHeaders=0x2000"), "{error}");
        assert!(error.contains("section table end=0x1B0"), "{error}");
    }
}
