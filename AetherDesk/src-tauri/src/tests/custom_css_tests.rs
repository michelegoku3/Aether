use crate::core::custom_css::{custom_css_path, ensure_custom_css, read_custom_css};
use std::fs;
use std::sync::{Mutex, OnceLock};

static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[test]
fn custom_css_path_is_inside_config() {
    let path = custom_css_path();
    assert!(path.ends_with("custom.css"));
    assert!(path.to_string_lossy().contains("config"));
}

#[test]
fn read_missing_file_returns_empty() {
    let _guard = lock();
    let path = custom_css_path();
    // Ensure we start from a clean state for this test (fail-open: remove if exists from previous run)
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir(path.parent().unwrap());
    let content = read_custom_css().expect("read should not error on missing file");
    assert_eq!(content, "");
}

#[test]
fn ensure_creates_template_and_read_returns_it() {
    let _guard = lock();
    let path = ensure_custom_css().expect("ensure should succeed");
    assert!(path.exists());
    let content = read_custom_css().expect("read after ensure");
    assert!(content.contains("AetherDesk Custom CSS"));
    assert!(content.contains("--bg-app"));
    // Cleanup: remove file and parent dir so other tests see "missing" state
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir(path.parent().unwrap());
}

#[test]
fn custom_css_enabled_defaults_to_false() {
    use crate::core::settings::AppSettings;
    let s = AppSettings::default();
    assert!(!s.custom_css_enabled);
}
