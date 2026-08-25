//! Detection of SteamStub on a game executable.
//!
//! Port of UCOnline2 `patch.bat` `:detect_steamstub` (v1.19.6+): a PE with a
//! `.bind` section that carries one of the known SteamStub 1.x–3.x loader
//! signatures. Used to pre-check `[Settings] GetStubbedLol` in the UI.

use std::path::Path;

/// SteamStub 3.x / 64-bit prologue (call $+5; push regs; push r8).
const SIG_STUB64: &[u8] = &[
    0xe8, 0x00, 0x00, 0x00, 0x00, 0x50, 0x53, 0x51, 0x52, 0x56, 0x57, 0x55, 0x41, 0x50,
];
/// SteamStub 3.x 32-bit variant.
const SIG_STUB3: &[u8] = &[
    0xe8, 0x00, 0x00, 0x00, 0x00, 0x50, 0x53, 0x51, 0x52, 0x56, 0x57, 0x55, 0x8b, 0x44,
    0x24, 0x1c, 0x2d, 0x05, 0x00, 0x00, 0x00, 0x8b, 0xcc, 0x83, 0xe4, 0xf0, 0x51, 0x51,
    0x51, 0x50,
];
/// SteamStub 2.x.
const SIG_STUB2: &[u8] = &[
    0x53, 0x51, 0x52, 0x56, 0x57, 0x55, 0x8b, 0xec, 0x81, 0xec, 0x00, 0x10, 0x00, 0x00,
];
/// SteamStub 1.x.
const SIG_STUB1: &[u8] = &[0x60, 0x81, 0xec, 0x00, 0x10, 0x00, 0x00, 0xbe];

/// True when `exe` looks like a SteamStub-protected PE.
pub fn detect_steamstub(exe: &Path) -> bool {
    let Ok(bytes) = std::fs::read(exe) else {
        return false;
    };
    detect_steamstub_bytes(&bytes)
}

/// Same check on an already-read PE image (for tests).
pub fn detect_steamstub_bytes(bytes: &[u8]) -> bool {
    let Some(bind) = bind_section_payload(bytes) else {
        return false;
    };
    contains_seq(&bind, SIG_STUB64)
        || contains_seq(&bind, SIG_STUB3)
        || contains_seq(&bind, SIG_STUB2)
        || contains_seq(&bind, SIG_STUB1)
}

fn bind_section_payload(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 256 || bytes[0] != b'M' || bytes[1] != b'Z' {
        return None;
    }
    let pe = u32::from_le_bytes(bytes.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if pe + 24 >= bytes.len() || bytes.get(pe..pe + 2) != Some(b"PE") {
        return None;
    }
    let section_count = u16::from_le_bytes(bytes.get(pe + 6..pe + 8)?.try_into().ok()?) as usize;
    let optional_size = u16::from_le_bytes(bytes.get(pe + 20..pe + 22)?.try_into().ok()?) as usize;
    let table = pe + 24 + optional_size;

    for index in 0..section_count {
        let offset = table + 40 * index;
        let header = bytes.get(offset..offset + 40)?;
        let name = header.get(..8)?;
        let is_bind = name.starts_with(b".bind") && name.iter().skip(5).all(|&b| b == 0);
        if !is_bind {
            continue;
        }
        let raw_size = u32::from_le_bytes(header.get(16..20)?.try_into().ok()?) as usize;
        let raw_ptr = u32::from_le_bytes(header.get(20..24)?.try_into().ok()?) as usize;
        if raw_ptr >= bytes.len() {
            return None;
        }
        let take = raw_size.min(bytes.len() - raw_ptr);
        return Some(&bytes[raw_ptr..raw_ptr + take]);
    }
    None
}

fn contains_seq(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_not_steamstub() {
        assert!(!detect_steamstub_bytes(b"not a pe"));
        assert!(!detect_steamstub_bytes(&[0u8; 300]));
    }

    #[test]
    fn bind_section_with_stub1_signature_is_detected() {
        let mut image = vec![0u8; 0x200];
        image[0] = b'M';
        image[1] = b'Z';
        image[0x3c] = 0x80;
        image[0x80] = b'P';
        image[0x81] = b'E';
        // section count at pe+6
        image[0x86] = 1;
        image[0x87] = 0;
        // optional header size at pe+20 = 0 → section table at 0x98
        image[0x94] = 0;
        image[0x95] = 0;
        // section name ".bind"
        image[0x98..0x9d].copy_from_slice(b".bind");
        // raw size at header+16
        let raw_size: u32 = SIG_STUB1.len() as u32;
        image[0x98 + 16..0x98 + 20].copy_from_slice(&raw_size.to_le_bytes());
        // raw ptr at header+20
        let raw_ptr: u32 = 0x180;
        image[0x98 + 20..0x98 + 24].copy_from_slice(&raw_ptr.to_le_bytes());
        image[0x180..0x180 + SIG_STUB1.len()].copy_from_slice(SIG_STUB1);
        assert!(detect_steamstub_bytes(&image));
    }
}
