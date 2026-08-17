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
        match manager.fetch_latest_desk_test_release().await {
            Ok(release) => {
                if GithubReleaseManager::latest_is_newer_than(&current_version, &release.tag_name) {
                    crate::desk_log_info!("updater", "AetherDesk test update available: {} (current: {})", release.tag_name, current_version);
                    return prepare_from_release(&release).await.map(Some);
                } else {
                    crate::desk_log_info!("updater", "AetherDesk test release {} is not newer than installed version {}", release.tag_name, current_version);
                    return Ok(None);
                }
            }
            Err(error) => {
                crate::desk_log_warn!("updater", "AetherDesk test release lookup failed during install: {}", error);
            }
        }
    }

    let release = manager.fetch_latest_desk_release().await?;
    if !GithubReleaseManager::latest_is_newer_than(&current_version, &release.tag_name) {
        crate::desk_log_info!("updater", "AetherDesk stable release {} is not newer than installed version {}", release.tag_name, current_version);
        return Ok(None);
    }

    crate::desk_log_info!("updater", "AetherDesk stable update available: {} (current: {})", release.tag_name, current_version);
    prepare_from_release(&release).await.map(Some)
}

/// Scarica e prepara l'ultima release STABILE di AetherDesk senza gate di
/// versione: usata per uscire dal canale test ("Restore" nella UI) tornando
/// alla build stabile precedente. Il processo viene poi riavviato dal chiamante.
pub async fn prepare_stable_restore() -> Result<Option<PreparedUpdate>, String> {
    let manager = GithubReleaseManager::new();
    let release = manager.fetch_latest_desk_release().await?;
    crate::desk_log_info!(
        "updater",
        "Restoring latest stable AetherDesk release: {} (leaving test channel)",
        release.tag_name
    );
    prepare_from_release(&release).await.map(Some)
}

/// Downloads and extracts a given desk release's portable ZIP into staging.
async fn prepare_from_release(release: &GithubRelease) -> Result<PreparedUpdate, String> {
    let asset = GithubReleaseManager::find_desk_zip_asset(release)?;
    crate::desk_log_info!(
        "updater",
        "Staging AetherDesk {} from {} ({})",
        release.tag_name,
        asset.browser_download_url,
        asset.name
    );
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

// ---------------------------------------------------------------------------
// Portable self-uninstall
// ---------------------------------------------------------------------------
//
// Same constraint as the updater: a running `.exe` cannot delete its own
// directory on Windows. We stage a copy of the binary in the *system* temp
// folder (never inside install_root / AetherData — both may be deleted) and
// launch it with `--uninstall-desk` so it can wait for the lock and wipe the
// folder after the main process exits.
//
// Extra Windows hardness:
// - the main process exits via `std::process::exit` (not only `app.exit`) so
//   WebView2 cannot keep the process alive in the background;
// - the helper force-kills leftover `AetherDesk.exe` instances before delete;
// - folder wipe clears contents first, then removes the root (avoids the
//   classic "empty folder still locked by cwd" leftover).

const UNINSTALL_HELPER_NAME: &str = "aether_uninstaller.exe";
const DATA_DIR_NAME: &str = "AetherData";
const DESK_PROCESS_NAME: &str = "aetherdesk.exe";

fn uninstall_helper_path() -> PathBuf {
    std::env::temp_dir().join(UNINSTALL_HELPER_NAME)
}

/// Stages and launches the external uninstaller helper.
///
/// The caller **must** hard-exit the running instance afterwards so the install
/// folder (and `AetherDesk.exe`) become deletable.
///
/// - `delete_user_data = true`  → remove the whole install folder (incl. AetherData)
/// - `delete_user_data = false` → move `AetherData` to the parent of install_root,
///   then remove the install folder
pub fn schedule_uninstall(delete_user_data: bool) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve current exe: {e}"))?;
    let install_root = LocalAppPaths::install_root();
    let helper = uninstall_helper_path();

    if let Some(parent) = helper.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to prepare uninstaller temp dir: {e}"))?;
    }
    // Overwrite any stale helper from a previous attempt.
    let _ = fs::remove_file(&helper);
    fs::copy(&current_exe, &helper)
        .map_err(|e| format!("Failed to stage uninstaller helper: {e}"))?;

    let mut cmd = Command::new(&helper);
    // Run from system temp so the helper's cwd cannot lock install_root.
    if let Some(temp_parent) = helper.parent() {
        cmd.current_dir(temp_parent);
    }
    cmd.arg("--uninstall-desk")
        .arg(install_root.to_string_lossy().as_ref());
    if delete_user_data {
        cmd.arg("--delete-user-data");
    } else {
        cmd.arg("--keep-user-data");
    }

    // Detach helper so it survives the hard exit of the parent process.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS | CREATE_NO_WINDOW);
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to launch uninstaller helper: {e}"))?;

    crate::desk_log_info!(
        "lifecycle",
        "Scheduled portable uninstall (delete_user_data={}, install_root={})",
        delete_user_data,
        install_root.display()
    );
    Ok(())
}

/// Runs as the external `--uninstall-desk` helper (from system temp).
pub fn run_uninstall(install_root: &Path, delete_user_data: bool) -> i32 {
    // Give the parent a moment to begin exiting after spawning us.
    thread::sleep(Duration::from_millis(800));

    // Force-kill any leftover AetherDesk.exe (main app / WebView host) that
    // would keep directory handles open. Never touches aether_uninstaller.exe.
    force_kill_desk_processes();
    thread::sleep(Duration::from_millis(600));

    let exe = install_root.join("AetherDesk.exe");
    if exe.exists() {
        // Prefer waiting for a clean unlock; if it never unlocks, kill again
        // and continue — we still want to wipe as much as possible.
        if !wait_until_replaceable(&exe) {
            eprintln!("[AetherDesk] exe still locked after wait; force-killing again");
            force_kill_desk_processes();
            thread::sleep(Duration::from_millis(800));
            let _ = wait_until_replaceable(&exe);
        }
    }

    // Icon restore must run against a path that *survives* uninstall:
    // - keep user data → move AetherData first, then write official shell.ico there
    // - wipe everything → write official shell.ico to a small durable AppData cache
    //   so Start Menu / Desktop shortcuts stop pointing at a deleted custom icon
    let mut preserved_data: Option<PathBuf> = None;
    if !delete_user_data {
        match preserve_user_data(install_root) {
            Ok(dest) => preserved_data = dest,
            Err(e) => {
                eprintln!("[AetherDesk] failed to preserve AetherData: {e}");
                // Abort so the user does not lose data if the move failed.
                return 1;
            }
        }
    }

    if let Err(e) = restore_default_icon_after_uninstall(preserved_data.as_deref()) {
        eprintln!("[AetherDesk] failed to restore default shell icon before wipe: {e}");
    }

    match wipe_install_root(install_root) {
        Ok(()) => {
            eprintln!(
                "[AetherDesk] uninstall complete: removed {}",
                install_root.display()
            );
            schedule_helper_self_delete();
            0
        }
        Err(e) => {
            eprintln!(
                "[AetherDesk] failed to remove {}: {e}",
                install_root.display()
            );
            // Last resort on Windows: cmd rmdir after another kill pass.
            force_kill_desk_processes();
            thread::sleep(Duration::from_millis(500));
            if shell_rmdir(install_root) {
                eprintln!(
                    "[AetherDesk] uninstall complete via shell rmdir: {}",
                    install_root.display()
                );
                schedule_helper_self_delete();
                0
            } else {
                1
            }
        }
    }
}

/// Terminates every running process named `AetherDesk.exe` (case-insensitive).
/// The helper binary is `aether_uninstaller.exe`, so it is never targeted.
fn force_kill_desk_processes() {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    let self_pid = sysinfo::get_current_pid().ok();

    for (pid, process) in sys.processes() {
        if self_pid == Some(*pid) {
            continue;
        }
        let name = process.name().to_lowercase();
        if name == DESK_PROCESS_NAME {
            eprintln!("[AetherDesk] killing leftover process {} (pid={})", name, pid);
            let _ = process.kill();
        }
    }

    // Also hit taskkill as a belt-and-suspenders path for stubborn children
    // that sysinfo may not enumerate the same way (msedgewebview2 is separate
    // but the main lock is AetherDesk.exe itself).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "AetherDesk.exe", "/T"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
}

/// Moves `<install_root>/AetherData` to `<install_root>/../AetherData`
/// (with numeric suffix if the destination already exists).
/// Returns the final destination when a move happened, or `None` if there
/// was nothing to preserve.
fn preserve_user_data(install_root: &Path) -> Result<Option<PathBuf>, String> {
    let data_dir = install_root.join(DATA_DIR_NAME);
    if !data_dir.exists() {
        return Ok(None);
    }

    let parent = install_root.parent().ok_or_else(|| {
        format!(
            "Install root {} has no parent; cannot relocate AetherData",
            install_root.display()
        )
    })?;

    let dest = unique_sibling_dir(parent, DATA_DIR_NAME);
    // Prefer rename (atomic on same volume). Fall back to copy+delete.
    match fs::rename(&data_dir, &dest) {
        Ok(()) => {}
        Err(rename_err) => {
            copy_dir_recursive(&data_dir, &dest).map_err(|e| {
                format!(
                    "Failed to relocate {} → {} (rename: {rename_err}; copy: {e})",
                    data_dir.display(),
                    dest.display()
                )
            })?;
            let _ = fs::remove_dir_all(&data_dir);
        }
    }
    eprintln!(
        "[AetherDesk] preserved user data at {}",
        dest.display()
    );
    Ok(Some(dest))
}

/// Point Start Menu / Desktop shortcuts back at the official AetherDesk icon.
///
/// `preserved_data` is the relocated `AetherData` folder when the user kept
/// their data; otherwise a durable AppData cache path is used so shortcuts
/// do not keep referencing a deleted custom `shell.ico`.
fn restore_default_icon_after_uninstall(preserved_data: Option<&Path>) -> Result<(), String> {
    let shell_ico = if let Some(data_root) = preserved_data {
        let dest = data_root.join("config").join("icons").join("shell.ico");
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create icons folder: {e}"))?;
        }
        fs::write(&dest, crate::core::custom_css::DEFAULT_AETHER_ICON)
            .map_err(|e| format!("Failed to write default shell.ico: {e}"))?;
        dest
    } else {
        // Full wipe: keep a tiny durable icon outside the deleted install tree.
        let cache = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("AetherDesk");
        fs::create_dir_all(&cache)
            .map_err(|e| format!("Failed to create icon cache folder: {e}"))?;
        let dest = cache.join("shell.ico");
        fs::write(&dest, crate::core::custom_css::DEFAULT_AETHER_ICON)
            .map_err(|e| format!("Failed to write default shell.ico: {e}"))?;
        dest
    };

    crate::core::shell_shortcuts::sync_windows_shortcuts(&shell_ico);
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("create {}: {e}", dst.display()))?;
    for entry in fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("entry in {}: {e}", src.display()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
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

fn unique_sibling_dir(parent: &Path, base_name: &str) -> PathBuf {
    let candidate = parent.join(base_name);
    if !candidate.exists() {
        return candidate;
    }
    for i in 1..1000 {
        let alt = parent.join(format!("{base_name}-{i}"));
        if !alt.exists() {
            return alt;
        }
    }
    parent.join(format!(
        "{base_name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ))
}

/// Wipes install_root thoroughly: delete children first, then the root itself.
fn wipe_install_root(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let mut last_err = String::new();
    for attempt in 0..50 {
        // Clear children so a locked cwd on the empty root is less likely.
        if path.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    let child = entry.path();
                    // Best-effort child cleanup; root remove below owns errors.
                    if child.is_dir() {
                        let _ = fs::remove_dir_all(&child);
                    } else {
                        let _ = fs::remove_file(&child);
                    }
                }
            }
        }

        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(_) if !path.exists() => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                // On stubborn locks, re-kill desk processes mid-retry.
                if attempt == 10 || attempt == 25 {
                    force_kill_desk_processes();
                }
                thread::sleep(Duration::from_millis(200 + (attempt as u64) * 40));
            }
        }
    }

    // Success if nothing is left (even if the last call returned an error).
    if !path.exists() {
        return Ok(());
    }
    // Empty dir left behind still counts as failure — try one last remove_dir.
    if path.is_dir() {
        if let Ok(mut entries) = fs::read_dir(path) {
            if entries.next().is_none() {
                if fs::remove_dir(path).is_ok() || !path.exists() {
                    return Ok(());
                }
            }
        }
    }
    Err(last_err)
}

/// Windows `rmdir /s /q` fallback when Rust fs APIs cannot finish the job.
fn shell_rmdir(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let status = Command::new("cmd")
            .args(["/C", "rmdir", "/s", "/q"])
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        match status {
            Ok(s) if s.success() => !path.exists(),
            _ => !path.exists(),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Best-effort delayed self-delete of the helper sitting in system temp.
fn schedule_helper_self_delete() {
    let helper = uninstall_helper_path();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        // ping ≈ 3s delay without needing timeout.exe / PowerShell policies.
        let cmdline = format!(
            "ping 127.0.0.1 -n 4 > nul & del /F /Q \"{}\"",
            helper.display()
        );
        let _ = Command::new("cmd")
            .args(["/C", &cmdline])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = fs::remove_file(&helper);
    }
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
    crate::desk_log_info!("updater", "Downloading AetherDesk zip from {}", url);
    let response = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "AetherDesk-Updater")
        .send()
        .await
        .map_err(|e| {
            crate::desk_log_error!("updater", "AetherDesk download network error: {e}");
            format!("Failed to reach download server: {e}")
        })?;

    crate::desk_log_info!("updater", "AetherDesk download HTTP {}", response.status());
    if !response.status().is_success() {
        crate::desk_log_error!(
            "updater",
            "AetherDesk download failed: HTTP {} from {}",
            response.status(),
            url
        );
        return Err(format!("Download server returned HTTP error: {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| {
            crate::desk_log_error!("updater", "AetherDesk download body error: {e}");
            format!("Failed to read downloaded bytes: {e}")
        })?;
    crate::desk_log_info!("updater", "AetherDesk zip size={} bytes", bytes.len());

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
