use crate::core::custom_css::{
    active_theme_name, active_wallpaper_name, content_hash, ensure_default_assets,
    personal_wallpaper_path, read_personal_wallpaper_data_uri, read_theme_css, sync_default_theme,
    theme_kit_dir, theme_path, themes_dir, wallpapers_dir, DefaultThemeOutcome,
};
use std::collections::BTreeMap;
use std::fs;
use std::sync::{Mutex, OnceLock};

const CYBERPUNK_THEME: &str = include_str!("../../assets/defaults/themes/cyberpunk.css");

static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

/// Tests write into the real (test-binary-relative) AetherData folder;
/// always clean the appearance folders afterwards so runs stay idempotent.
fn cleanup_appearance_dirs() {
    let _ = fs::remove_dir_all(themes_dir());
    let _ = fs::remove_dir_all(wallpapers_dir());
    let _ = fs::remove_dir_all(theme_kit_dir());
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
    assert!(themes_dir().join("halo.css").is_file());
    assert!(themes_dir().join("wonderland.css").is_file());
    assert!(theme_kit_dir().join("default.css").is_file());
    assert!(wallpapers_dir().join("cyberpunk.jpg").is_file());
    assert!(wallpapers_dir().join("goldmine.jpg").is_file());
    assert!(wallpapers_dir().join("frieren.jpg").is_file());
    assert!(wallpapers_dir().join("halo.jpg").is_file());

    cleanup_appearance_dirs();
}

#[test]
fn theme_kit_is_the_live_stylesheet_and_is_refreshed_on_start() {
    let _guard = lock();
    cleanup_appearance_dirs();

    ensure_default_assets().expect("seeding should succeed");

    let kit = theme_kit_dir().join("default.css");
    let seeded = fs::read_to_string(&kit).expect("the kit stylesheet should be readable");
    assert_eq!(
        seeded,
        include_str!("../../../src/style.css"),
        "the kit must ship the stylesheet the app itself loads"
    );

    // A stale or edited copy never survives the next start: the kit is a
    // reference, and a theme author copies it out before editing.
    fs::write(&kit, "/* stale */").expect("fixture write should succeed");
    ensure_default_assets().expect("seeding should succeed");
    assert_eq!(
        fs::read_to_string(&kit).expect("the kit stylesheet should be readable"),
        seeded
    );

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
    assert_eq!(
        name, "cyberpunk.css",
        "first detected theme should be cyberpunk.css"
    );

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
    assert_eq!(
        name, "cyberpunk.jpg",
        "first detected wallpaper should be cyberpunk.jpg"
    );

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

#[test]
fn traversal_selections_fall_back_to_first_detected() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    // Tampered settings must never escape the themes folder: each of these
    // resolves to None inside `resolve_selected`, so `theme_path` falls back
    // to the first detected file instead of an outside path.
    for evil in [
        "../settings.json",
        "..\\settings.json",
        "..",
        "sub/dir.css",
        "sub\\dir.css",
        "/etc/hostname",
        "C:\\Windows\\win.ini",
    ] {
        let path = theme_path(evil).expect("theme_path should succeed");
        let name = path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string());
        assert_eq!(
            name.as_deref(),
            Some("cyberpunk.css"),
            "evil selection: {evil}"
        );
        // Belt and braces: no resolved path may leave the themes dir.
        if let Some(p) = &path {
            assert!(
                p.starts_with(themes_dir()),
                "escaped themes dir: {}",
                p.display()
            );
        }
    }

    cleanup_appearance_dirs();
}

#[test]
fn defaults_manifest_records_every_seeded_theme() {
    let _guard = lock();
    cleanup_appearance_dirs();

    ensure_default_assets().expect("seeding should succeed");

    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(themes_dir().join(".aether-defaults.json"))
            .expect("manifest should be readable"),
    )
    .expect("manifest should be valid JSON");
    let themes = manifest["themes"]
        .as_object()
        .expect("manifest should carry theme hashes");

    for name in [
        "cyberpunk.css",
        "frieren.css",
        "goldmine.css",
        "halo.css",
        "wonderland.css",
    ] {
        let bytes = fs::read(themes_dir().join(name)).expect("theme should be readable");
        assert_eq!(
            themes[name].as_str(),
            Some(content_hash(&bytes).as_str()),
            "{name} should be tracked with the hash of the copy we wrote"
        );
    }

    cleanup_appearance_dirs();
}

#[test]
fn untouched_default_theme_is_refreshed_on_upgrade() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    // Stand-in for a theme an older release wrote: the manifest holds its
    // hash, so the next start may replace it with the current default.
    let path = themes_dir().join("cyberpunk.css");
    let previous = "/* previous release */";
    fs::write(&path, previous).expect("fixture write should succeed");
    let mut tracked = BTreeMap::from([(
        "cyberpunk.css".to_string(),
        content_hash(previous.as_bytes()),
    )]);

    assert_eq!(
        sync_default_theme("cyberpunk.css", CYBERPUNK_THEME, &mut tracked, &[]),
        DefaultThemeOutcome::Refreshed
    );
    assert_eq!(
        fs::read_to_string(&path).expect("theme should be readable"),
        CYBERPUNK_THEME
    );
    assert_eq!(
        tracked["cyberpunk.css"],
        content_hash(CYBERPUNK_THEME.as_bytes())
    );

    cleanup_appearance_dirs();
}

#[test]
fn edited_default_theme_is_never_overwritten() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    let path = themes_dir().join("cyberpunk.css");
    let custom = "/* my own night city */\n:root { --cyber-yellow: #ff0000; }";
    fs::write(&path, custom).expect("fixture write should succeed");

    ensure_default_assets().expect("reseeding should succeed");

    assert_eq!(
        fs::read_to_string(&path).expect("theme should be readable"),
        custom,
        "a file the user edited must survive every start"
    );

    cleanup_appearance_dirs();
}

#[test]
fn legacy_143_theme_is_adopted_on_first_upgrade() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    // An installation upgrading from 1.4.3 has the old copy but no manifest:
    // the legacy hash is the only evidence that the file is still a seed.
    fs::remove_file(themes_dir().join(".aether-defaults.json"))
        .expect("fixture cleanup should succeed");
    let path = themes_dir().join("cyberpunk.css");
    let legacy = "/* 1.4.3 copy */";
    fs::write(&path, legacy).expect("fixture write should succeed");
    let hash = content_hash(legacy.as_bytes());
    let mut tracked = BTreeMap::new();

    assert_eq!(
        sync_default_theme(
            "cyberpunk.css",
            CYBERPUNK_THEME,
            &mut tracked,
            &[("cyberpunk.css", hash.as_str())],
        ),
        DefaultThemeOutcome::Refreshed
    );
    assert_eq!(
        fs::read_to_string(&path).expect("theme should be readable"),
        CYBERPUNK_THEME
    );

    cleanup_appearance_dirs();
}

/// Removes `/* ... */` blocks so a comment's punctuation cannot split a
/// declaration in the parsers below.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Token names declared in the first `:root` block of a stylesheet.
fn root_tokens(css: &str) -> Vec<String> {
    let css = strip_comments(css);
    let Some(start) = css.find(":root") else {
        return Vec::new();
    };
    let Some(open) = css[start..].find('{') else {
        return Vec::new();
    };
    let Some(close) = css[start + open..].find('}') else {
        return Vec::new();
    };
    css[start + open + 1..start + open + close]
        .split(';')
        .filter_map(|declaration| {
            let name = declaration.split(':').next()?.trim();
            name.starts_with("--").then(|| name.to_string())
        })
        .collect()
}

#[test]
fn default_themes_define_every_shared_token() {
    const STYLESHEET: &str = include_str!("../../../src/style.css");
    // Geometry and the live-preview switches are component defaults, not
    // palette: a theme has no reason to touch them.
    const RUNTIME_TOKENS: &[&str] = &["--sidebar-width", "--alt-card-opacity", "--alt-card-fade"];

    let base = root_tokens(STYLESHEET);
    assert!(
        base.len() > 50,
        "stylesheet should expose the shared palette"
    );

    // Wonderland is intentionally absent: it replaces the component look with
    // its own --tv-* tokens instead of remapping the shared palette.
    let themes = [
        (
            "cyberpunk.css",
            include_str!("../../assets/defaults/themes/cyberpunk.css"),
        ),
        (
            "frieren.css",
            include_str!("../../assets/defaults/themes/frieren.css"),
        ),
        (
            "goldmine.css",
            include_str!("../../assets/defaults/themes/goldmine.css"),
        ),
        (
            "halo.css",
            include_str!("../../assets/defaults/themes/halo.css"),
        ),
    ];

    for (name, css) in themes {
        let defined = root_tokens(css);
        let missing: Vec<&String> = base
            .iter()
            .filter(|token| !RUNTIME_TOKENS.contains(&token.as_str()))
            .filter(|token| !defined.contains(token))
            .collect();
        assert!(
            missing.is_empty(),
            "{name} must remap every shared token; missing: {missing:?}"
        );
    }
}

#[test]
fn oversized_wallpaper_is_refused_with_a_clear_error() {
    let _guard = lock();
    cleanup_appearance_dirs();
    ensure_default_assets().expect("seeding should succeed");

    // 9 MiB of zeros with a valid extension: over the 8 MiB cap.
    let big = wallpapers_dir().join("huge.jpg");
    fs::write(&big, vec![0_u8; 9 * 1024 * 1024]).expect("fixture write should succeed");
    let error = read_personal_wallpaper_data_uri("huge.jpg").unwrap_err();
    assert!(error.contains("too large"), "unexpected error: {error}");

    // A normal wallpaper still loads as a data URI.
    let uri = read_personal_wallpaper_data_uri("").expect("read should succeed");
    assert!(uri.is_some());
    assert!(uri.unwrap().starts_with("data:image/"));

    cleanup_appearance_dirs();
}
