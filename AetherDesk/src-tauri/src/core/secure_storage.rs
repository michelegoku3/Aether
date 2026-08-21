//! Current-user secret protection for local AetherDesk state.
//!
//! Windows DPAPI keeps credentials and backend keys unreadable at rest while
//! avoiding application-managed encryption keys. Callers own serialization and
//! atomic file I/O; this module owns only protection and memory cleanup.

#[cfg(target_os = "windows")]
pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let success = unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 {
        return Err("Windows could not protect local secret data".to_string());
    }
    take_dpapi_output(output)
}

#[cfg(target_os = "windows")]
pub fn unprotect(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let success = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 {
        return Err("Windows could not unlock local secret data".to_string());
    }
    take_dpapi_output(output)
}

#[cfg(target_os = "windows")]
fn take_dpapi_output(
    output: windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB,
) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;

    if output.pbData.is_null() {
        return Err("Windows DPAPI returned an empty output buffer".to_string());
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
    };
    unsafe { LocalFree(output.pbData as _) };
    Ok(bytes)
}

#[cfg(not(target_os = "windows"))]
pub fn protect(_plain: &[u8]) -> Result<Vec<u8>, String> {
    Err("Secure local state storage is currently available only on Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn unprotect(_encrypted: &[u8]) -> Result<Vec<u8>, String> {
    Err("Secure local state storage is currently available only on Windows".to_string())
}
