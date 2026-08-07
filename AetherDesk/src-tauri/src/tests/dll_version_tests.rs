use crate::updater::dll_version::{version_from_fixed_header, read_installed_dll_version};
use crate::updater::dll::AETHER_DLL_FILES;

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
    let dir = std::env::temp_dir().join(format!("aether_dllver_missing_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    assert_eq!(read_installed_dll_version(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_without_version_resource_yield_no_version() {
    let dir = std::env::temp_dir().join(format!("aether_dllver_noresource_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    for file_name in AETHER_DLL_FILES {
        std::fs::write(dir.join(file_name), b"not a real PE").expect("write fake dll");
    }
    assert_eq!(read_installed_dll_version(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
}
