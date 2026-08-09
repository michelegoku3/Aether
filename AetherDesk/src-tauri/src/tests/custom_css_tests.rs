use crate::core::custom_css::{
    active_theme_name, active_wallpaper_name, ensure_default_assets, personal_wallpaper_path,
    read_theme_css, theme_path, themes_dir, wallpapers_dir,
};
use std::fs;
use std::sync::{Mutex, OnceLock};

static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

/// Tests write into the real (test-binary-relative) AetherData folder;
/// always clean the appearance folders afterwards so runs stay idempotent.
fn cleanup_appearance_dirs() {
    let _ = fs::remove_dir_all(themes_dir());
    let _ = fs::remove_dir_all(wallpapers_dir());
}

#[test]
fn appearance_dirs_are_inside_config() {
    let themes = themes_dir();
    let wallpapers = wallpapers_dir();
    assert!(themes.to_string_lossy().contains("config"));
    assert!(wallpapers.to_string_lossy().contains("config"));
    assert!(themes.ends_with("themes"));
    assert!(wallpapers.ends_with("wallpapers"));
}

#[test]
fn ensure_default_assets_creates_dirs_and_seeds_defaults() {
    let _guard = lock();
    cleanup_appearance_dirs();

    ensure_default_assets().expect("seeding should succeed");

    assert!(themes_dir().is_dir());
    assert!(wallpapers_dir().is_dir());
    assert!(themes_dir().join("cyberpunk.css").is_file());
    assert!(themes_dir().join("goldmine.css").is_file());
    assert!(themes_dir().join("frieren.css").is_file());
    assert!(wallpapers_dir().join("cyberpunk.jpg").is_file());
    assert!(wallpapers_dir().join("frieren.jpg").is_file());

    cleanup_appearance_dirs();
}

#[test]
fn first_theme_is_cyberpunk_alphabetically() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    let path = theme_path("").expect("theme_path should succeed");
    assert!(path.is_some());
    let name = path
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(name, "cyberpunk.css", "first detected theme should be cyberpunk.css");

    assert_eq!(active_theme_name("").as_deref(), Some("cyberpunk.css"));

    cleanup_appearance_dirs();
}

#[test]
fn read_theme_css_returns_content() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    let css = read_theme_css("").expect("read_theme_css should succeed");
    assert!(!css.trim().is_empty());
    assert!(css.contains(":root"), "theme should define :root variables");

    cleanup_appearance_dirs();
}

#[test]
fn first_wallpaper_is_cyberpunk_alphabetically() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    let path = personal_wallpaper_path("").expect("wallpaper path should succeed");
    assert!(path.is_some());
    let name = path
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(name, "cyberpunk.jpg", "first detected wallpaper should be cyberpunk.jpg");

    assert_eq!(active_wallpaper_name("").as_deref(), Some("cyberpunk.jpg"));

    cleanup_appearance_dirs();
}

#[test]
fn explicit_selection_wins_when_file_exists() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    // Pick the Frieren theme explicitly: it must be returned over the
    // alphabetically-first cyberpunk one.
    let path = theme_path("frieren.css").expect("theme_path should succeed");
    assert_eq!(
        path.map(|p| p.file_name().unwrap().to_string_lossy().to_string()),
        Some("frieren.css".to_string())
    );

    // A selection pointing to a missing file falls back to the first theme.
    let path = theme_path("does-not-exist.css").expect("theme_path should succeed");
    assert_eq!(
        path.map(|p| p.file_name().unwrap().to_string_lossy().to_string()),
        Some("cyberpunk.css".to_string())
    );

    cleanup_appearance_dirs();
}

#[test]
fn empty_dirs_yield_no_theme_and_no_wallpaper() {
    let _guard = lock();
    cleanup_appearance_dirs();

    // Recreate empty folders so the check runs against them.
    let _ = fs::create_dir_all(themes_dir());
    let _ = fs::create_dir_all(wallpapers_dir());

    assert!(theme_path("").expect("theme_path should succeed").is_none());
    assert!(personal_wallpaper_path("")
        .expect("wallpaper path should succeed")
        .is_none());

    cleanup_appearance_dirs();
}
