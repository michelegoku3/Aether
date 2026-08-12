//! Portable self-updater for AetherDesk.
//!
//! AetherDesk is distributed as a portable ZIP (no NSIS installer, no registry,
//! no Start Menu entries). Updating therefore cannot rely on `tauri-plugin-updater`
//! (which drives an NSIS installer). Instead the app downloads the latest portable
//! ZIP from a GitHub release, extracts it next to the running binary, and swaps
//! the files in place.
//!
//! # Why a copy of the binary is used as the "updater"
//!
//! A running `.exe` cannot be overwritten on Windows, so the in-place swap must
//! happen in a process that is *not* the one being replaced. To keep the whole
//! update logic inside this crate (DRY, one source of truth) and to avoid fragile
//! shell/batch escaping, we:
//!
//! 1. download + extract the new ZIP into a staging folder ([`prepare_update`]);
//! 2. copy the *current* `AetherDesk.exe` to a sibling `aether_updater.exe`
//!    and launch it with `--apply-update <staging> <install_root>`
//!    ([`schedule_restart`]);
//! 3. exit the running instance (freeing the original exe file lock).
//!
//! The copy (running from the temp path) then waits for the original exe to be
//! unlocked, swaps the files in, cleans up and relaunches `AetherDesk.exe`.
//!
//! # Safety invariants
//! - `AetherData` (user config, themes, wallpapers, backups) is **never** touched:
//!   the release ZIP does not contain it, and the swap skips it explicitly.
//! - Extraction uses `enclosed_name()` to prevent path-traversal from a malicious
//!   ZIP (defense in depth, even though releases come only from the private repo).

use crate::core::paths::LocalAppPaths;
use crate::updater::github::{GithubRelease, GithubReleaseManager};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// A fully staged update, ready to be applied after the current instance exits.
pub struct PreparedUpdate {
    /// Directory that directly contains the extracted `AetherDesk.exe`.
    pub app_root: PathBuf,
    /// The live install directory (`LocalAppPaths::install_root()`).
    pub install_root: PathBuf,
}

/// Root of every temporary artifact produced during an update.
fn update_workdir() -> PathBuf {
    LocalAppPaths::temp_dir().join("desk_update")
}

/// Downloads and extracts the latest portable ZIP into the staging folder.
///
/// Update-source selection:
///   - If testing releases are enabled, the *test* desk release (`tdesk-*`)
///     takes priority and is staged whenever present (presence = update).
///   - Otherwise the latest *stable* desk release (`desk-*`) is used, gated on
///     its version being newer than the installed one.
///
/// Returns `Ok(None)` when no update applies, `Ok(Some(...))` once a new build
/// is fully staged. Real failures return `Err`. If the caller receives
/// `Some(...)` it must call [`schedule_restart`] and then exit the running
/// instance so the swap can proceed.
pub async fn prepare_update(app: &tauri::AppHandle) -> Result<Option<PreparedUpdate>, String> {
    let current_version = app.package_info().version.to_string();
    let manager = GithubReleaseManager::new();

    // Testing updates take priority when enabled. Their version is gated by
    // `latest_is_newer_than`, exactly like stable releases: if the test release
    // is not newer than installed, we return Ok(None) (up to date).
    let settings = crate::core::settings::SettingsManager::new(app).load();
    if settings.enable_test_updates {
        if let Ok(release) = manager.fetch_latest_desk_test_release().await {
            if GithubReleaseManager::latest_is_newer_than(&current_version, &release.tag_name) {
                crate::desk_log_info!("updater", "AetherDesk test update available: {} (current: {})", release.tag_name, current_version);
                return prepare_from_release(&release).await.map(Some);
            } else {
                crate::desk_log_info_once!("updater", "AetherDesk test release {} is not newer than installed version {}", release.tag_name, current_version);
                return Ok(None);
            }
        }
    }

    let release = manager.fetch_latest_desk_release().await?;
    if !GithubReleaseManager::latest_is_newer_than(&current_version, &release.tag_name) {
        crate::desk_log_info_once!("updater", "AetherDesk stable release {} is not newer than installed version {}", release.tag_name, current_version);
        return Ok(None);
    }

    crate::desk_log_info!("updater", "AetherDesk stable update available: {} (current: {})", release.tag_name, current_version);
    prepare_from_release(&release).await.map(Some)
}

/// Downloads and extracts a given desk release's portable ZIP into staging.
async fn prepare_from_release(release: &GithubRelease) -> Result<PreparedUpdate, String> {
    let asset = GithubReleaseManager::find_desk_zip_asset(release)?;
    let workdir = update_workdir();
    fs::create_dir_all(&workdir)
        .map_err(|e| format!("Failed to create update workdir {}: {e}", workdir.display()))?;

    let zip_path = workdir.join("aetherdesk_update.zip");
    download_zip(&asset.browser_download_url, &zip_path).await?;

    let staging = workdir.join("staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)
        .map_err(|e| format!("Failed to create staging dir {}: {e}", staging.display()))?;

    let app_root = extract_zip(&zip_path, &staging)?;

    Ok(PreparedUpdate {
        app_root,
        install_root: LocalAppPaths::install_root(),
    })
}

/// Spawns a copy of the current exe that will perform the swap, then must be
/// followed by exiting the running instance.
pub fn schedule_restart(update: &PreparedUpdate) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve current exe: {e}"))?;

    let updater_path = update_workdir().join("aether_updater.exe");
    fs::copy(&current_exe, &updater_path)
        .map_err(|e| format!("Failed to stage updater copy: {e}"))?;

    Command::new(&updater_path)
        .arg("--apply-update")
        .arg(update.app_root.to_string_lossy().as_ref())
        .arg(update.install_root.to_string_lossy().as_ref())
        .spawn()
        .map_err(|e| format!("Failed to launch updater: {e}"))?;

    Ok(())
}

/// Applies a staged update. Runs as the `--apply-update` process (a copy of the
/// binary living in the temp folder), so it can replace the original exe.
pub fn run_apply_update(staging: &Path, install_root: &Path) -> i32 {
    let exe = install_root.join("AetherDesk.exe");
    let old = install_root.join("AetherDesk.exe.old");

    // Wait until the original exe is no longer locked by the exiting instance.
    if !wait_until_replaceable(&exe) {
        eprintln!("[AetherDesk] timed out waiting for the old exe to unlock");
        return 1;
    }

    // Rename the old exe so the new one can take its place.
    let _ = fs::rename(&exe, &old);

    // Merge staging into the install root, never touching AetherData.
    if let Err(e) = merge_tree(staging, install_root) {
        eprintln!("[AetherDesk] update apply failed: {e}");
        let _ = fs::rename(&old, &exe);
        return 1;
    }

    // Cleanup staging and the downloaded zip (the temp `aether_updater.exe` is
    // left in place because it is the currently running process).
    let _ = fs::remove_file(old);
    let _ = fs::remove_dir_all(staging);
    let _ = fs::remove_file(update_workdir().join("aetherdesk_update.zip"));

    // Relaunch the freshly updated app.
    let _ = Command::new(&exe).spawn();
    0
}

/// Removes any leftover update artifacts from a previous, possibly interrupted
/// self-update (staging, zip, stale updater copy). Safe to call on normal app
/// startup because in that context no updater process is running.
pub fn cleanup_stale_artifacts() {
    let _ = fs::remove_dir_all(update_workdir());
}

/// Polls until `path` can be renamed (i.e. no process holds it open).
fn wait_until_replaceable(path: &Path) -> bool {
    let probe = path.with_extension("exe.lock");
    for _ in 0..150 {
        match fs::rename(path, &probe) {
            Ok(()) => {
                let _ = fs::rename(&probe, path);
                return true;
            }
            Err(_) => thread::sleep(Duration::from_millis(200)),
        }
    }
    false
}

/// Downloads `url` into `dest_path` (atomic write via temp file).
async fn download_zip(url: &str, dest_path: &Path) -> Result<(), String> {
    let response = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "AetherDesk-Updater")
        .send()
        .await
        .map_err(|e| format!("Failed to reach download server: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("Download server returned HTTP error: {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read downloaded bytes: {e}"))?;

    let temp_path = dest_path.with_extension("tmp");
    fs::write(&temp_path, &bytes).map_err(|e| format!("Failed to write update ZIP: {e}"))?;
    fs::rename(&temp_path, dest_path).map_err(|e| format!("Failed to finalize update ZIP: {e}"))?;
    Ok(())
}

/// Extracts `zip_path` into `dest_dir` safely and returns the *app root*: the
/// directory that contains `AetherDesk.exe` directly (handles a possible
/// top-level folder inside the ZIP).
fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("Failed to open update ZIP: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Invalid update ZIP archive: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry {i}: {e}"))?;

        let relative = entry
            .enclosed_name()
            .ok_or_else(|| format!("Unsafe path in ZIP entry {i}"))?;

        let target = dest_dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|e| format!("Failed to create dir {}: {e}", target.display()))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create dir {}: {e}", parent.display()))?;
            }
            let mut out = fs::File::create(&target)
                .map_err(|e| format!("Failed to create file {}: {e}", target.display()))?;
            io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed to extract {}: {e}", target.display()))?;
        }
    }

    locate_app_root(dest_dir).ok_or_else(|| {
        format!(
            "Extracted ZIP does not contain AetherDesk.exe under {}",
            dest_dir.display()
        )
    })
}

/// Finds the directory (at most one level deep) that directly holds `AetherDesk.exe`.
fn locate_app_root(dest_dir: &Path) -> Option<PathBuf> {
    if dest_dir.join("AetherDesk.exe").is_file() {
        return Some(dest_dir.to_path_buf());
    }
    if let Ok(entries) = fs::read_dir(dest_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("AetherDesk.exe").is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Recursively merges `src` into `dst`, copying files and dirs, skipping the
/// user data folder (`AetherData`) so it can never be overwritten.
fn merge_tree(src: &Path, dst: &Path) -> Result<(), String> {
    for entry in fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("entry in {}: {e}", src.display()))?;
        let name = entry.file_name();
        // Guard: never touch user data.
        if name.eq_ignore_ascii_case("AetherData") {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            fs::create_dir_all(&to).map_err(|e| format!("create {}: {e}", to.display()))?;
            merge_tree(&from, &to)?;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            fs::copy(&from, &to)
                .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))?;
        }
    }
    Ok(())
}
