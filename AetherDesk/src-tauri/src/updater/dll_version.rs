//! Lettura della versione AetherDLL **direttamente dai file .dll** (PE version resource).
//!
//! # Perché esiste
//! Da quando la repo AetherDLL embedde una `VS_VERSION_INFO` resource in ogni binario
//! (root `CMakeLists.txt` → `common/version.rc.in`), la versione vive dentro i file
//! stessi — esattamente come la versione di AetherDesk vive dentro AetherDesk.exe
//! grazie a Tauri/tauri.conf.json. Questo modulo è il lato lettura: interroga la
//! resource dei 3 binari nella cartella Steam, **senza nessun file esterno**
//! (niente bookmark `.txt`, niente fingerprint `.json`).
//!
//! # Contratto
//! - `read_installed_dll_version` ritorna `Some("x.y.z")` SOLO se tutti e 3 i file
//!   esistono, hanno la resource e concordano sulla stessa versione (un'installazione
//!   "mista" con versioni diverse tra loro è ambigua → `None`, il chiamante ricade
//!   sulla catena legacy per installazioni pre-resource).
//! - Fuori da Windows la lettura è uno stub che ritorna sempre `None` (il dominio
//!   AetherDLL è Windows-only, ma il crate deve compilare ovunque).

use crate::updater::dll::AETHER_DLL_FILES;
use std::path::Path;

/// Dimensione minima della VS_FIXEDFILEINFO (13 DWORD = 52 byte), dalla documentazione
/// Microsoft: sotto questa soglia il blocco root non può essere una versione valida.
const FIXED_FILE_INFO_SIZE: u32 = 52;
/// dwSignature atteso di una VS_FIXEDFILEINFO valida.
const FIXED_FILE_INFO_SIGNATURE: u32 = 0xFEEF04BD;

/// Estrae `(major, minor, patch)` dai primi 4 DWORD di una VS_FIXEDFILEINFO.
/// Funzione pura, unit-testabile: il layout è la parte storicamente più fragile
/// (dwStrucVersion è 0x00010000 fisso e NON va confuso con la versione del file).
fn version_from_fixed_header(header: &[u32; 4]) -> Option<(u16, u16, u16)> {
    if header[0] != FIXED_FILE_INFO_SIGNATURE {
        return None;
    }
    let file_version_ms = header[2];
    let file_version_ls = header[3];
    Some((
        (file_version_ms >> 16) as u16,
        (file_version_ms & 0xFFFF) as u16,
        (file_version_ls >> 16) as u16,
    ))
}

/// Versione `(major, minor, patch)` letta dalla `VS_FIXEDFILEINFO` di un file PE.
/// `None` se il file non esiste, non ha version resource o la lettura fallisce.
#[cfg(windows)]
pub fn read_file_version(path: &Path) -> Option<(u16, u16, u16)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // "\" = blocco root: VerQueryValueW lo espone come VS_FIXEDFILEINFO.
    let root_block: Vec<u16> = "\\".encode_utf16().chain(Some(0)).collect();

    unsafe {
        let size = GetFileVersionInfoSizeW(wide_path.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None; // nessuna version resource nel file
        }

        let mut buffer = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide_path.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) == 0 {
            return None;
        }

        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut data_len: u32 = 0;
        if VerQueryValueW(buffer.as_ptr().cast(), root_block.as_ptr(), &mut data, &mut data_len) == 0
            || data.is_null()
            || data_len < FIXED_FILE_INFO_SIZE
        {
            return None;
        }

        version_from_fixed_header(&std::ptr::read_unaligned(data as *const [u32; 4]))
    }
}

/// Stub non-Windows: nessuna versione leggibile (il chiamante usa il fallback legacy).
#[cfg(not(windows))]
pub fn read_file_version(_path: &Path) -> Option<(u16, u16, u16)> {
    None
}

/// Versione concordata dei 3 binari AetherDLL installati nella directory di Steam.
///
/// Ritorna `Some("x.y.z")` solo con installazione completa e coerente (tutti i file
/// presenti, tutti con resource, tutti alla stessa versione). In ogni altro caso
/// `None`: manca file, manca resource (installazioni pre-resource) o versioni miste.
pub fn read_installed_dll_version(steam_dir: &Path) -> Option<String> {
    let mut agreed: Option<(u16, u16, u16)> = None;

    for file_name in AETHER_DLL_FILES {
        let version = read_file_version(&steam_dir.join(file_name))?;
        match agreed {
            None => agreed = Some(version),
            Some(existing) if existing == version => {}
            Some(_) => return None, // versioni diverse tra i file: stato ambiguo
        }
    }

    agreed.map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regressione del bug "1.0.0": dwStrucVersion (0x00010000) NON deve finire nel
    /// risultato. Il test replica i DWORD di una resource con versione 0.9.7 / 2.4.1.
    #[test]
    fn fixed_header_layout_reads_file_version_not_struc_version() {
        let header: [u32; 4] = [0xFEEF04BD, 0x00010000, (0 << 16) | 9, 7 << 16];
        assert_eq!(version_from_fixed_header(&header), Some((0, 9, 7)));

        let header: [u32; 4] = [0xFEEF04BD, 0x00010000, (2 << 16) | 4, 1 << 16];
        assert_eq!(version_from_fixed_header(&header), Some((2, 4, 1)));
    }

    #[test]
    fn fixed_header_rejects_bad_signature() {
        let header: [u32; 4] = [0xDEADBEEF, 0x00010000, 9, 7 << 16];
        assert_eq!(version_from_fixed_header(&header), None);
    }

    #[test]
    fn missing_files_yield_no_version() {
        let dir = std::env::temp_dir().join(format!(
            "aether_dllver_missing_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        // Nessun file presente: la lettura deve degradare a None, mai panic.
        assert_eq!(read_installed_dll_version(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_without_version_resource_yield_no_version() {
        let dir = std::env::temp_dir().join(format!(
            "aether_dllver_noresource_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test dir");
        for file_name in AETHER_DLL_FILES {
            std::fs::write(dir.join(file_name), b"not a real PE").expect("write fake dll");
        }
        assert_eq!(read_installed_dll_version(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
