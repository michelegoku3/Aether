use crate::core::migration::migrate_legacy_steam_proxy_with_check;
use crate::updater::dll::{DllInstaller, AETHER_DLL_FILES};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct SteamFixture(PathBuf);
impl SteamFixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "aether-xinput-install-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("steam.exe"), b"test Steam root").unwrap();
        Self(root)
    }
    fn installer(&self) -> DllInstaller {
        DllInstaller::new(self.0.to_string_lossy().into_owned())
    }
    fn zip(&self, entries: &[&str]) -> PathBuf {
        let path = self.0.join("release.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for entry in entries {
            writer.start_file(*entry, options).unwrap();
            writer.write_all(b"new Aether DLL").unwrap();
        }
        writer.finish().unwrap();
        path
    }
    fn root(&self) -> &Path { &self.0 }
}
impl Drop for SteamFixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

#[test]
fn release_installs_only_three_dlls_beside_steam_exe() {
    let f = SteamFixture::new();
    let zip = f.zip(&[
        "out/Release/AetherCore.dll", "out/Release/AetherPayload.dll",
        "out/Release/xinput1_4.dll",
    ]);
    let installer = f.installer();
    installer.install_from_zip(&zip).unwrap();
    assert!(installer.verify_installation());
    for name in AETHER_DLL_FILES {
        assert!(f.root().join(name).is_file());
        assert!(!f.root().join("bin").join(name).exists());
    }
    installer.uninstall().unwrap();
    assert!(!installer.verify_installation());
    for name in AETHER_DLL_FILES {
        assert!(!f.root().join(name).exists());
    }
}

#[test]
fn startup_proxy_migration_is_deferred_while_steam_runs_then_idempotent() {
    let f = SteamFixture::new();
    let legacy = f.root().join("dwmapi.dll");
    fs::write(&legacy, b"old proxy").unwrap();
    let path = f.root().to_str().unwrap();

    let error = migrate_legacy_steam_proxy_with_check(path, || true).unwrap_err();
    assert!(error.contains("deferred"), "{error}");
    assert_eq!(fs::read(&legacy).unwrap(), b"old proxy");
    assert!(migrate_legacy_steam_proxy_with_check(path, || false).unwrap());
    assert!(!legacy.exists());
    assert!(!migrate_legacy_steam_proxy_with_check(path, || panic!("no file: no process scan needed")).unwrap());
}

#[test]
fn startup_proxy_migration_requires_a_validated_steam_root() {
    let f = SteamFixture::new();
    let legacy = f.root().join("dwmapi.dll");
    fs::write(&legacy, b"leave this file alone").unwrap();
    fs::remove_file(f.root().join("steam.exe")).unwrap();
    assert!(!migrate_legacy_steam_proxy_with_check(
        f.root().to_str().unwrap(),
        || panic!("invalid root: no process scan needed"),
    ).unwrap());
    assert!(f.installer().migrate_legacy_proxy().is_err());
    assert!(legacy.exists());
}

#[test]
fn install_migrates_proxy_only_after_validating_release() {
    let f = SteamFixture::new();
    let legacy = f.root().join("dwmapi.dll");
    fs::write(&legacy, b"old proxy").unwrap();
    fs::write(f.root().join("AetherCore.dll"), b"previous Core").unwrap();
    let installer = f.installer();
    assert_eq!(installer.count_aether_residuals(), 2);
    let invalid_zip = f.zip(&["AetherCore.dll", "AetherPayload.dll"]);
    assert!(installer.install_from_zip(&invalid_zip).is_err());
    assert!(legacy.exists());
    assert_eq!(fs::read(f.root().join("AetherCore.dll")).unwrap(), b"previous Core");

    let valid_zip = f.zip(&AETHER_DLL_FILES);
    installer.install_from_zip(&valid_zip).unwrap();
    assert!(!legacy.exists());
    assert!(installer.verify_installation());
    assert!(!installer.migrate_legacy_proxy().unwrap());
}

#[test]
fn uninstall_and_reset_also_remove_proxy_left_after_startup() {
    let f = SteamFixture::new();
    let legacy = f.root().join("dwmapi.dll");
    let installer = f.installer();
    let path = f.root().to_str().unwrap();
    fs::write(&legacy, b"old proxy").unwrap();
    assert!(migrate_legacy_steam_proxy_with_check(path, || true).is_err());
    installer.uninstall().unwrap();
    assert!(!legacy.exists());
    fs::write(&legacy, b"old proxy").unwrap();
    assert!(migrate_legacy_steam_proxy_with_check(path, || true).is_err());
    assert_eq!(installer.reset_aether_files().unwrap(), 1);
    assert!(!legacy.exists());
    assert_eq!(installer.count_aether_residuals(), 0);
}

#[test]
fn legacy_proxy_removal_failure_rolls_back_supported_dlls() {
    let f = SteamFixture::new();
    // A directory at the old filename simulates an undeletable file on all OSes.
    let legacy = f.root().join("dwmapi.dll");
    fs::create_dir(&legacy).unwrap();
    for name in AETHER_DLL_FILES {
        fs::write(f.root().join(name), b"old").unwrap();
    }
    let zip = f.zip(&AETHER_DLL_FILES);
    let installer = f.installer();
    assert!(installer.install_from_zip(&zip).is_err());
    for name in AETHER_DLL_FILES {
        assert_eq!(fs::read(f.root().join(name)).unwrap(), b"old");
        assert!(!f.root().join(format!("{name}.aether.bak")).exists());
        assert!(!f.root().join(format!("{name}.aether.tmp")).exists());
    }
    assert!(legacy.is_dir());
    assert!(installer.uninstall().is_err());
    assert!(installer.reset_aether_files().is_err());
}

#[test]
fn release_missing_xinput_never_overwrites_the_existing_core() {
    let f = SteamFixture::new();
    fs::write(f.root().join("AetherCore.dll"), b"previous good version").unwrap();
    let zip = f.zip(&["AetherCore.dll", "AetherPayload.dll"]);
    let error = f.installer().install_from_zip(&zip).unwrap_err();
    assert!(error.contains("xinput1_4.dll"), "{error}");
    assert_eq!(fs::read(f.root().join("AetherCore.dll")).unwrap(), b"previous good version");
}

#[test]
fn release_with_extra_dll_is_rejected_before_replacing_any_file() {
    let f = SteamFixture::new();
    let zip = f.zip(&[
        "AetherCore.dll", "AetherPayload.dll", "xinput1_4.dll", "extra/unsupported.dll",
    ]);
    let error = f.installer().install_from_zip(&zip).unwrap_err();
    assert!(error.contains("Unsupported DLL"), "{error}");
    assert!(!f.root().join("AetherCore.dll").exists());
}

#[test]
fn locked_or_invalid_last_proxy_rolls_back_all_previous_files() {
    let f = SteamFixture::new();
    for name in &AETHER_DLL_FILES[..2] {
        fs::write(f.root().join(name), b"old").unwrap();
    }
    // A directory occupying the proxy filename is a deterministic failure,
    // including on filesystems where locking DLLs cannot be simulated.
    fs::create_dir(f.root().join("xinput1_4.dll")).unwrap();
    let zip = f.zip(&AETHER_DLL_FILES);
    assert!(f.installer().install_from_zip(&zip).is_err());
    for name in &AETHER_DLL_FILES[..2] {
        assert_eq!(fs::read(f.root().join(name)).unwrap(), b"old");
        assert!(!f.root().join(format!("{name}.aether.bak")).exists());
        assert!(!f.root().join(format!("{name}.aether.tmp")).exists());
    }
    assert!(f.root().join("xinput1_4.dll").is_dir());
}

#[test]
fn duplicate_proxy_entry_is_rejected_before_replacing_any_dll() {
    let f = SteamFixture::new();
    let zip = f.zip(&[
        "AetherCore.dll", "AetherPayload.dll",
        "xinput1_4.dll", "second/xinput1_4.dll",
    ]);
    let error = f.installer().install_from_zip(&zip).unwrap_err();
    assert!(error.contains("Duplicate"), "{error}");
    assert!(!f.root().join("AetherCore.dll").exists());
}
