//! Conservative portable uninstall: explicit files, never a recursive parent wipe.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

// Only byte-identical bundled files are owned. Modified/unknown files stay put.
const BUNDLED_FILES: &[(&str, &str)] = &[
    (
        "ExternalTools/Steamless/Plugins/ExamplePlugin.dll",
        "c6dce0ce7d432fbb8cea249e4dcd1394adc8c2adc819012c745b709c68e585a9",
    ),
    (
        "ExternalTools/Steamless/Plugins/SharpDisasm.dll",
        "6d3ce990cdf58da228697d25416d16d15994135c5f66571fe1e00e9c975bc2cf",
    ),
    (
        "ExternalTools/Steamless/Plugins/Steamless.API.dll",
        "d6acc4b0cc768213a46ffad0a6bf6070a6b13f79a22e0588f0ab50c950f9248c",
    ),
    (
        "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant10.x86.dll",
        "c890dcda2e79970e5a5541ee6340e503e829982f1613c36119be7f754c32065d",
    ),
    (
        "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant20.x86.dll",
        "3cca1fb90b1aa3e1703565a095b665acfc8073b889ff44ef97d17f4221fe707f",
    ),
    (
        "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant21.x86.dll",
        "2f92eeaf47accd0f87bbe7fb7cf85fcd3c00830f464dfdbd4f661449bbc92b21",
    ),
    (
        "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant30.x64.dll",
        "81f1d36540fc8756b155b3e58b34f54da4f9bc1f974988cf91d0b17ffc1d14d8",
    ),
    (
        "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant30.x86.dll",
        "fe2102b096d44aa956788d067856225254ee4e43c133c939e3cd5c0a6cc37399",
    ),
    (
        "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant31.x64.dll",
        "790f1974f97258058cb57c20787e8a2fcb5c16cca0911719b698580d74e38918",
    ),
    (
        "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant31.x86.dll",
        "fc7e1ac80a7d2c0121e083d218b7369c4d217eccaf395cd1c0292d000f19446b",
    ),
    (
        "ExternalTools/Steamless/Steamless.CLI.exe",
        "70cd54354865ede605ec0fbfadf15f5302aa85a777394f28b0de6acfd243e795",
    ),
    (
        "ExternalTools/Steamless/Steamless.CLI.exe.config",
        "27bc6422c238e69933f5a986518e3a0077c748bd9420c98530403b8f5b2dcacb",
    ),
    (
        "ExternalTools/Steamless/Steamless.exe",
        "7d8858454e2065fac487ef23a5cc5f1b7e7c54656ca949c7931c8a17079afb2f",
    ),
    (
        "ExternalTools/Steamless/Steamless.exe.config",
        "27bc6422c238e69933f5a986518e3a0077c748bd9420c98530403b8f5b2dcacb",
    ),
    (
        "ExternalTools/UCOnline2/VERSION",
        "2139da5fac112157c6b025f44e7e83e90eee345c852ccb3a76fd2a2f3ecdeb45",
    ),
    (
        "ExternalTools/UCOnline2/plugins/EOS_custom.dll",
        "306029beb4d6d2ccb9270fed23f4b6de5ad6a7a1c7420d814a66e21e82777389",
    ),
    (
        "ExternalTools/UCOnline2/plugins/coherence_universal.dll",
        "8a034257c909581f4b328604ffe43b115adb6bc172dba5d8e23e4aec01f22115",
    ),
    (
        "ExternalTools/UCOnline2/plugins/overlay_proxy.dll",
        "629de6c450063a68ceb2cb3675876b6c625265af1be98ed14a74d969aa661d81",
    ),
    (
        "ExternalTools/UCOnline2/plugins/photon_universal.dll",
        "9ff8dc4d2d4380aa76f9f0072f46c9524bafb8603f698cae5dcb72afdcbd2e8a",
    ),
    (
        "ExternalTools/UCOnline2/plugins/playfab_universal.dll",
        "7ddcb49329bc30e328edb9bb5ebff9f7ad6872c83b9c1927a78887e60e3507af",
    ),
    (
        "ExternalTools/UCOnline2/plugins/r5.dll",
        "80318e3f6a24f0682c62f31ef864dd761eb6acf5a43771d84aef2f2a0004d42e",
    ),
    (
        "ExternalTools/UCOnline2/plugins/raft_mp.dll",
        "ad5e883870a292c293a0e10342d44055ca3c80a44d646c234903dc8dd2c0fc56",
    ),
    (
        "ExternalTools/UCOnline2/plugins/unity_auth_bypass.dll",
        "f48b05d09fd419c6a094810faff7b97f17db387c351b1c7a9ccbbb02fb32d81b",
    ),
    (
        "ExternalTools/UCOnline2/x64/steam_api64.dll",
        "f0739db59a06ec6af6dc411328a986256a9b33101a6092b05650bf57e1ae281f",
    ),
    (
        "ExternalTools/UCOnline2/x86/steam_api.dll",
        "6b03755e2b33c1e994206a8ed60c10a0aea0546c49e63425562ef0dba330e9df",
    ),
];

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("read {}: {e}", path.display()))?;
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

fn check_no_links(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("inspect {}: {error}", ancestor.display())),
        };
        let linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            linked || metadata.file_attributes() & 0x400 != 0
        };
        if linked {
            return Err(format!(
                "Refusing linked/reparse path {}",
                ancestor.display()
            ));
        }
    }
    Ok(())
}

fn validate_root(
    root: &Path,
    expected_exe_hash: &str,
    protected: &[PathBuf],
) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("Uninstall requires an absolute install path".into());
    }
    check_no_links(root)?;
    let root = fs::canonicalize(root).map_err(|e| format!("resolve {}: {e}", root.display()))?;
    if root.parent().is_none() || protected.iter().any(|p| p == &root) {
        return Err(format!(
            "Refusing protected uninstall root {}",
            root.display()
        ));
    }
    let exe = root.join("AetherDesk.exe");
    check_no_links(&exe)?;
    if hash_file(&exe)? != expected_exe_hash {
        return Err(format!(
            "AetherDesk.exe does not match this helper in {}",
            root.display()
        ));
    }
    Ok(root)
}

pub(super) fn validate_target(root: &Path) -> Result<PathBuf, String> {
    let helper = std::env::current_exe().map_err(|e| e.to_string())?;
    let protected: Vec<_> = [
        dirs::home_dir(),
        dirs::desktop_dir(),
        dirs::download_dir(),
        dirs::document_dir(),
        dirs::data_dir(),
        dirs::data_local_dir(),
        dirs::config_dir(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|p| fs::canonicalize(p).ok())
    .collect();
    validate_root(root, &hash_file(&helper)?, &protected)
}

fn validate_tree(root: &Path) -> Result<(), String> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        check_no_links(&path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
                pending.push(entry.map_err(|e| e.to_string())?.path());
            }
        } else if !metadata.is_file() {
            return Err(format!("Refusing special file {}", path.display()));
        }
    }
    Ok(())
}

fn remove_verified_file(path: &Path, expected: &str) -> Result<(), String> {
    for attempt in 0..50 {
        check_no_links(path)?;
        if !path.try_exists().map_err(|e| e.to_string())? {
            return Ok(());
        }
        if hash_file(path)? != expected {
            return Err(format!("File changed before removal: {}", path.display()));
        }
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied && attempt < 49 => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(error) => return Err(format!("remove {}: {error}", path.display())),
        }
    }
    unreachable!("the final removal attempt returns")
}

fn remove_owned_files(root: &Path, delete_user_data: bool, exe_hash: &str) -> Result<(), String> {
    let mut files = Vec::new();
    for &(relative, expected) in BUNDLED_FILES {
        let path = root.join(relative);
        check_no_links(&path)?;
        if path.try_exists().map_err(|e| e.to_string())? && hash_file(&path)? == expected {
            files.push((path, expected.to_string()));
        }
    }
    // Check the full opted-in data tree before any removal. Kept data is never moved.
    let data = root.join("AetherData");
    check_no_links(&data)?;
    if data.try_exists().map_err(|e| e.to_string())? {
        validate_tree(&data)?;
    }
    if delete_user_data && data.exists() {
        check_no_links(&data)?;
        fs::remove_dir_all(&data).map_err(|e| format!("remove {}: {e}", data.display()))?;
    }
    files.push((root.join("AetherDesk.exe"), exe_hash.to_string()));
    let mut directories = BTreeSet::new();
    for (path, expected) in files {
        remove_verified_file(&path, &expected)?;
        for parent in path.ancestors().skip(1).take_while(|p| *p != root) {
            directories.insert(parent.to_path_buf());
        }
    }
    let mut directories: Vec<_> = directories.into_iter().collect();
    directories.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    directories.push(root.to_path_buf());
    for directory in directories {
        check_no_links(&directory)?;
        match fs::remove_dir(&directory) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(format!("remove empty {}: {error}", directory.display())),
        }
    }
    Ok(())
}

pub(super) fn uninstall(root: &Path, delete_user_data: bool) -> Result<(), String> {
    let root = validate_target(root)?;
    let helper = std::env::current_exe().map_err(|e| e.to_string())?;
    remove_owned_files(&root, delete_user_data, &hash_file(&helper)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn installation() -> (TempDir, PathBuf, String) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("mixed");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("AetherDesk.exe"), b"owned executable").unwrap();
        fs::write(root.join("sentinel.txt"), b"unrelated").unwrap();
        fs::create_dir_all(root.join("AetherData/config")).unwrap();
        fs::write(root.join("AetherData/config/sentinel.txt"), b"user data").unwrap();
        let hash = hash_file(&root.join("AetherDesk.exe")).unwrap();
        (temp, root, hash)
    }

    #[test]
    fn uninstall_keeps_unrelated_files_and_retained_data_in_place() {
        let (_temp, root, hash) = installation();
        let root = validate_root(&root, &hash, &[]).unwrap();
        remove_owned_files(&root, false, &hash).unwrap();
        assert!(!root.join("AetherDesk.exe").exists());
        assert_eq!(fs::read(root.join("sentinel.txt")).unwrap(), b"unrelated");
        assert_eq!(
            fs::read(root.join("AetherData/config/sentinel.txt")).unwrap(),
            b"user data"
        );
    }

    #[test]
    fn uninstall_deletes_opted_in_data_but_not_unrelated_files() {
        let (_temp, root, hash) = installation();
        let root = validate_root(&root, &hash, &[]).unwrap();
        remove_owned_files(&root, true, &hash).unwrap();
        assert!(!root.join("AetherData").exists());
        assert!(!root.join("AetherDesk.exe").exists());
        assert_eq!(fs::read(root.join("sentinel.txt")).unwrap(), b"unrelated");
    }

    #[test]
    fn modified_tool_is_not_owned_and_is_kept() {
        let (_temp, root, hash) = installation();
        let version = root.join("ExternalTools/UCOnline2/VERSION");
        fs::create_dir_all(version.parent().unwrap()).unwrap();
        fs::write(&version, b"unrelated modified file").unwrap();
        remove_owned_files(&root, false, &hash).unwrap();
        assert_eq!(fs::read(version).unwrap(), b"unrelated modified file");
    }

    #[test]
    fn protected_and_mismatched_roots_are_rejected_before_removal() {
        let (_temp, root, hash) = installation();
        let root = fs::canonicalize(root).unwrap();
        assert!(validate_root(&root, &hash, std::slice::from_ref(&root)).is_err());
        assert!(validate_root(&root, "different executable", &[]).is_err());
        assert_eq!(fs::read(root.join("sentinel.txt")).unwrap(), b"unrelated");
        assert!(root.join("AetherDesk.exe").is_file());
    }

    #[test]
    fn unchanged_bundle_is_removed_and_empty_install_directory_is_removed() {
        let (_temp, root, hash) = installation();
        fs::remove_file(root.join("sentinel.txt")).unwrap();
        let version = root.join("ExternalTools/UCOnline2/VERSION");
        fs::create_dir_all(version.parent().unwrap()).unwrap();
        fs::write(
            &version,
            include_bytes!("../../ExternalTools/UCOnline2/VERSION"),
        )
        .unwrap();
        remove_owned_files(&root, true, &hash).unwrap();
        assert!(!root.exists());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn linked_data_aborts_without_touching_external_sentinels() {
        let (_temp, root, hash) = installation();
        fs::remove_dir_all(root.join("AetherData")).unwrap();
        let outside = root.parent().unwrap().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel.txt"), b"outside").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("AetherData")).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(root.join("AetherData"))
            .arg(&outside)
            .status()
            .unwrap()
            .success());
        assert!(remove_owned_files(&root, true, &hash).is_err());
        assert!(root.join("AetherDesk.exe").is_file());
        assert_eq!(fs::read(outside.join("sentinel.txt")).unwrap(), b"outside");
    }

    #[test]
    fn bundle_manifest_matches_the_packaged_files() {
        let files: &[(&str, &[u8])] = &[
            (
                "ExternalTools/Steamless/Plugins/ExamplePlugin.dll",
                include_bytes!("../../ExternalTools/Steamless/Plugins/ExamplePlugin.dll"),
            ),
            (
                "ExternalTools/Steamless/Plugins/SharpDisasm.dll",
                include_bytes!("../../ExternalTools/Steamless/Plugins/SharpDisasm.dll"),
            ),
            (
                "ExternalTools/Steamless/Plugins/Steamless.API.dll",
                include_bytes!("../../ExternalTools/Steamless/Plugins/Steamless.API.dll"),
            ),
            (
                "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant10.x86.dll",
                include_bytes!(
                    "../../ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant10.x86.dll"
                ),
            ),
            (
                "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant20.x86.dll",
                include_bytes!(
                    "../../ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant20.x86.dll"
                ),
            ),
            (
                "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant21.x86.dll",
                include_bytes!(
                    "../../ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant21.x86.dll"
                ),
            ),
            (
                "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant30.x64.dll",
                include_bytes!(
                    "../../ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant30.x64.dll"
                ),
            ),
            (
                "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant30.x86.dll",
                include_bytes!(
                    "../../ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant30.x86.dll"
                ),
            ),
            (
                "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant31.x64.dll",
                include_bytes!(
                    "../../ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant31.x64.dll"
                ),
            ),
            (
                "ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant31.x86.dll",
                include_bytes!(
                    "../../ExternalTools/Steamless/Plugins/Steamless.Unpacker.Variant31.x86.dll"
                ),
            ),
            (
                "ExternalTools/Steamless/Steamless.CLI.exe",
                include_bytes!("../../ExternalTools/Steamless/Steamless.CLI.exe"),
            ),
            (
                "ExternalTools/Steamless/Steamless.CLI.exe.config",
                include_bytes!("../../ExternalTools/Steamless/Steamless.CLI.exe.config"),
            ),
            (
                "ExternalTools/Steamless/Steamless.exe",
                include_bytes!("../../ExternalTools/Steamless/Steamless.exe"),
            ),
            (
                "ExternalTools/Steamless/Steamless.exe.config",
                include_bytes!("../../ExternalTools/Steamless/Steamless.exe.config"),
            ),
            (
                "ExternalTools/UCOnline2/VERSION",
                include_bytes!("../../ExternalTools/UCOnline2/VERSION"),
            ),
            (
                "ExternalTools/UCOnline2/plugins/EOS_custom.dll",
                include_bytes!("../../ExternalTools/UCOnline2/plugins/EOS_custom.dll"),
            ),
            (
                "ExternalTools/UCOnline2/plugins/coherence_universal.dll",
                include_bytes!("../../ExternalTools/UCOnline2/plugins/coherence_universal.dll"),
            ),
            (
                "ExternalTools/UCOnline2/plugins/overlay_proxy.dll",
                include_bytes!("../../ExternalTools/UCOnline2/plugins/overlay_proxy.dll"),
            ),
            (
                "ExternalTools/UCOnline2/plugins/photon_universal.dll",
                include_bytes!("../../ExternalTools/UCOnline2/plugins/photon_universal.dll"),
            ),
            (
                "ExternalTools/UCOnline2/plugins/playfab_universal.dll",
                include_bytes!("../../ExternalTools/UCOnline2/plugins/playfab_universal.dll"),
            ),
            (
                "ExternalTools/UCOnline2/plugins/r5.dll",
                include_bytes!("../../ExternalTools/UCOnline2/plugins/r5.dll"),
            ),
            (
                "ExternalTools/UCOnline2/plugins/raft_mp.dll",
                include_bytes!("../../ExternalTools/UCOnline2/plugins/raft_mp.dll"),
            ),
            (
                "ExternalTools/UCOnline2/plugins/unity_auth_bypass.dll",
                include_bytes!("../../ExternalTools/UCOnline2/plugins/unity_auth_bypass.dll"),
            ),
            (
                "ExternalTools/UCOnline2/x64/steam_api64.dll",
                include_bytes!("../../ExternalTools/UCOnline2/x64/steam_api64.dll"),
            ),
            (
                "ExternalTools/UCOnline2/x86/steam_api.dll",
                include_bytes!("../../ExternalTools/UCOnline2/x86/steam_api.dll"),
            ),
        ];
        assert_eq!(files.len(), BUNDLED_FILES.len());
        for (&(name, bytes), &(relative, expected)) in files.iter().zip(BUNDLED_FILES) {
            assert_eq!(name, relative);
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                expected,
                "{relative}"
            );
        }
    }
}
