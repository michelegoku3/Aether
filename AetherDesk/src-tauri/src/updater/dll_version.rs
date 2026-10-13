//! Reads the AetherDLL version **directly from the .dll files** (PE version resource).
//!
//! # Why it exists
//! Since the AetherDLL repo embeds a `VS_VERSION_INFO` resource in every binary
//! (root `CMakeLists.txt` → `common/version.rc.in`), the version lives inside the
//! files themselves — exactly like the AetherDesk version lives inside AetherDesk.exe
//! thanks to Tauri/tauri.conf.json. This module is the read side: it queries the
//! resource of the 3 binaries of the XInput-only configuration in the Steam folder,
//! **with no external files**
//! (no `.txt` bookmarks, no `.json` fingerprints).
//!
//! # Contract
//! - `read_installed_dll_version` returns `Some("x.y.z")` ONLY when all 3 files
//!   exist, carry the resource, agree on the same version and no incompatible
//!   legacy component is present. Otherwise returns `None`.
//! - Off Windows the read is a stub that always returns `None` (the AetherDLL
//!   domain is Windows-only, but the crate must compile everywhere).

use crate::updater::dll::{DllInstaller, AETHER_DLL_FILES};
use std::path::Path;

/// Minimum size of a VS_FIXEDFILEINFO (13 DWORDs = 52 bytes), per Microsoft's
/// docs: below this threshold the root block cannot be a valid version.
#[cfg(windows)]
const FIXED_FILE_INFO_SIZE: u32 = 52;
/// Expected dwSignature of a valid VS_FIXEDFILEINFO.
#[cfg(any(windows, test))]
const FIXED_FILE_INFO_SIGNATURE: u32 = 0xFEEF04BD;

/// Extracts `(major, minor, patch)` from the first 4 DWORDs of a VS_FIXEDFILEINFO.
/// Pure, unit-testable function: the layout is the historically most fragile part
/// (dwStructVersion is fixed 0x00010000 and must NOT be confused with the file version).
///
/// It lives exactly where it is used: on Windows [`read_file_version`] calls it,
/// in every test build the pure tests in `tests::dll_version_tests` call it. On
/// non-Windows targets without tests it has no callers, so it is not compiled
/// (the `dead_code` lint would rightly flag it) — see the two constants above,
/// used only by its body.
#[cfg(any(windows, test))]
pub(crate) fn version_from_fixed_header(header: &[u32; 4]) -> Option<(u16, u16, u16)> {
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

/// `(major, minor, patch)` version read from the `VS_FIXEDFILEINFO` of a PE file.
/// `None` if the file does not exist, has no version resource, or reading fails.
#[cfg(windows)]
pub fn read_file_version(path: &Path) -> Option<(u16, u16, u16)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // "\" = root block: VerQueryValueW exposes it as VS_FIXEDFILEINFO.
    let root_block: Vec<u16> = "\\".encode_utf16().chain(Some(0)).collect();

    // SAFETY: all pointers passed to the Windows version-resource APIs point to
    // NUL-terminated buffers or writable storage owned by this function; the
    // returned data pointer is validated before it is read.
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
        if VerQueryValueW(
            buffer.as_ptr().cast(),
            root_block.as_ptr(),
            &mut data,
            &mut data_len,
        ) == 0
            || data.is_null()
            || data_len < FIXED_FILE_INFO_SIZE
        {
            return None;
        }

        version_from_fixed_header(&std::ptr::read_unaligned(data as *const [u32; 4]))
    }
}

/// Non-Windows stub: no readable PE version.
#[cfg(not(windows))]
pub fn read_file_version(_path: &Path) -> Option<(u16, u16, u16)> {
    None
}

/// Agreed version of the 3 XInput-only binaries in the Steam directory.
///
/// Returns `Some("x.y.z")` only with a complete, coherent install (every file
/// present, every file with the resource, every file at the same version).
/// `None` in every other case: missing file, missing resource (pre-resource
/// installs) or mixed versions.
pub fn read_installed_dll_version(steam_dir: &Path) -> Option<String> {
    if !DllInstaller::new(&steam_dir.to_string_lossy()).verify_installation() {
        return None;
    }
    let mut agreed: Option<(u16, u16, u16)> = None;

    for file_name in AETHER_DLL_FILES {
        let version = read_file_version(&steam_dir.join(file_name))?;
        match agreed {
            None => agreed = Some(version),
            Some(existing) if existing == version => {}
            Some(_) => return None, // files disagree: ambiguous state
        }
    }

    agreed.map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch))
}
