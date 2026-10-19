//! Tauri commands for UCOnline2 (Enable Online).
//!
//! Thin layer: resolves the installed game, locates the bundle and delegates
//! to the pure engine (`online::engine`). All heavy work runs on
//! `spawn_blocking` (the `commands/steamless.rs` pattern).

use crate::core::paths::LocalAppPaths;
use crate::core::presence_config::{aethercore_toml_paths, update_mode_in_toml, PresenceMode};
use crate::external_tools::bundle::ToolBundleLocator;
use crate::online::bundle::Uco2Bundle;
use crate::online::engine::OnlineEngine;
use crate::online::foreign::ForeignOnlineReport;
use crate::online::preferences::OnlinePreferencesStore;
use crate::online::state::OnlineStateStore;
use crate::online::types::{OnlineActionResult, OnlineEnableRequest, OnlinePlan, OnlineStatus};
use crate::util::game_resolver::resolve_installed_game;
use std::path::{Path, PathBuf};

const UCO2_RESOURCE_DIR: &str = "ExternalTools/UCOnline2";

/// UCOnline2 state file path (single source: `STATE_FILE`).
fn state_path() -> PathBuf {
    OnlineStateStore::state_path(&LocalAppPaths::data_root())
}

fn preferences_path() -> PathBuf {
    OnlinePreferencesStore::path(&LocalAppPaths::data_root())
}

#[tauri::command]
pub fn get_online_preferences(app_id: u32) -> Result<Option<OnlineEnableRequest>, String> {
    Ok(OnlinePreferencesStore::load(&preferences_path()).get(app_id))
}

#[tauri::command]
pub fn clear_online_preferences(app_id: u32) -> Result<(), String> {
    let path = preferences_path();
    OnlinePreferencesStore::load(&path).remove(app_id, &path)
}

/// A game's online state (for the UI: footer + record).
#[tauri::command]
pub async fn get_online_status(app: tauri::AppHandle, app_id: u32) -> Result<OnlineStatus, String> {
    let mut status = OnlineEngine::status(app_id, &state_path());
    if status.state != crate::online::types::OnlineStateKind::Enabled {
        // foreign_for_app scans the game filesystem:
        // spawn_blocking to avoid blocking the runtime.
        let foreign = tauri::async_runtime::spawn_blocking({
            let app = app.clone();
            move || foreign_for_app(&app, app_id)
        })
        .await
        .map_err(|e| format!("Online status task failed: {e}"))?;
        if foreign.uco2 {
            status.state = crate::online::types::OnlineStateKind::Enabled;
        }
    }
    Ok(status)
}

/// True when UCO2 is active for the game, verified by the presence of the
/// `union-crax.ini` file (written by UCO2 next to the running exe).
/// File-based detection, independent of Aether's state record.
/// Detection walks the whole game folder (walk+read) and must always run on
/// a blocking I/O thread, never on the tokio runtime.
#[tauri::command]
pub async fn is_uco2_active(app: tauri::AppHandle, app_id: u32) -> Result<bool, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);
    tauri::async_runtime::spawn_blocking(
        move || match crate::online::detect::GameInspector::inspect(&game_root) {
            Ok(detection) => Ok(detection.ini_dir.join("union-crax.ini").is_file()),
            Err(_) => Ok(false),
        },
    )
    .await
    .map_err(|e| format!("UCO2 check task failed: {e}"))?
}

/// Online-fix.me / SteamFix crack on the game disk (not UCO2, not Online Aether).
pub(crate) fn uco2_enabled(app_id: u32) -> bool {
    OnlineEngine::status(
        app_id,
        &OnlineStateStore::state_path(&LocalAppPaths::data_root()),
    )
    .state
        == crate::online::types::OnlineStateKind::Enabled
}

pub(crate) fn foreign_for_app(app: &tauri::AppHandle, app_id: u32) -> ForeignOnlineReport {
    let Ok(game) = resolve_installed_game(app, app_id) else {
        return ForeignOnlineReport::default();
    };
    crate::online::foreign::scan(Path::new(&game.game_path))
}

#[tauri::command]
pub async fn inspect_foreign_online(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<ForeignOnlineReport, String> {
    tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || Ok(foreign_for_app(&app, app_id))
    })
    .await
    .map_err(|e| format!("Foreign-online inspect task failed: {e}"))?
}

/// Activation plan (dry-run: no effect on disk).
#[tauri::command]
pub async fn plan_online(app: tauri::AppHandle, app_id: u32) -> Result<OnlinePlan, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);

    let bundle = locate_bundle(&app);
    let state_path = state_path();

    // Detection walks the game folder (heavy synchronous I/O):
    // spawn_blocking, never on the tokio runtime.
    let plan_root = game_root.clone();
    let mut plan = tauri::async_runtime::spawn_blocking(move || {
        OnlineEngine::plan(app_id, &plan_root, &bundle, &state_path)
    })
    .await
    .map_err(|e| format!("Plan task failed: {e}"))??;

    // "Game running" check (informational only in the plan).
    if let Some(exe) = &plan.detection.game_exe {
        if is_exe_running(exe) {
            plan.notices.push(format!(
                "The game appears to be running ({}): close it before enabling.",
                exe.file_name().and_then(|n| n.to_str()).unwrap_or("exe")
            ));
        }
    }

    crate::desk_log_info!(
        "online",
        "plan_online: app={} engine={:?} arch={:?} backends={:?} bundle_ok={} errors={:?}",
        app_id,
        plan.detection.engine,
        plan.detection.arch,
        plan.detection.backends,
        plan.prerequisites.bundle_ok,
        plan.prerequisites.errors
    );

    Ok(plan)
}

/// Enable UCOnline2 on a game (transactional deploy).
#[tauri::command]
pub async fn enable_online(
    app: tauri::AppHandle,
    app_id: u32,
    request: OnlineEnableRequest,
) -> Result<OnlineActionResult, String> {
    let steam_root = crate::commands::command_steam_path(&app)?;
    let mutation =
        crate::core::game_mutations::acquire(Path::new(&steam_root), app_id, "enable_online")
            .await?;
    crate::core::game_mutations::ensure_current_root(&app, Path::new(&steam_root))?;
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);
    let state_path = state_path();

    // Pre-flight: inspection walks the game tree (heavy synchronous I/O). It
    // goes on `spawn_blocking` like plan_online already did: never block the
    // tokio runtime for a folder scan.
    let preflight_root = game_root.clone();
    let preflight_state = state_path.clone();
    let inspection = tauri::async_runtime::spawn_blocking(move || {
        OnlineEngine::plan(
            app_id,
            &preflight_root,
            &Err("bundle not needed for inspection".to_string()),
            &preflight_state,
        )
    })
    .await
    .map_err(|e| format!("Pre-flight inspection task failed: {e}"))??;
    if let Some(exe) = &inspection.detection.game_exe {
        if is_exe_running(exe) {
            return Err(format!(
                "Close the game before enabling online ({}).",
                exe.file_name().and_then(|n| n.to_str()).unwrap_or("exe")
            ));
        }
    }

    let foreign = ForeignOnlineReport::from_conflicts(&inspection.detection.conflicts);
    if foreign.ofme {
        return Err(foreign.refuse_uco2());
    }

    let bundle = locate_bundle(&app)?;
    let backup_root = LocalAppPaths::backup_root();

    crate::desk_log_info!(
        "online",
        "enable_online: app={} ogAppId={} spoof={} eos_deploy={}",
        app_id,
        request.og_app_id,
        request.spoof_app_id,
        request.deploy_eos_custom
    );

    // Preferences have a longer lifecycle than the deployment journal. Save
    // before deployment so user input survives failures, closing the popup and
    // later disabling/re-enabling UCO2.
    let preferences_path = preferences_path();
    OnlinePreferencesStore::load(&preferences_path).upsert(
        app_id,
        request.clone(),
        &preferences_path,
    )?;

    let (_mutation, result) = tauri::async_runtime::spawn_blocking(move || {
        let result = OnlineEngine::enable(
            app_id,
            &game_root,
            &bundle,
            &request,
            &backup_root,
            &state_path,
        );
        (mutation, result)
    })
    .await
    .map_err(|e| format!("Online worker failed: {e}"))?;
    let result = result?;

    crate::desk_log_info!(
        "online",
        "enable_online done: success={} message='{}'",
        result.success,
        result.message
    );
    if result.success {
        for path in aethercore_toml_paths(&app) {
            update_mode_in_toml(&path, app_id, Some(PresenceMode::Excluded)).map_err(|e| {
                format!("Online files deployed, presence configuration incomplete: {e}")
            })?;
        }
    }
    Ok(result)
}

/// Disattiva UCOnline2 (rollback dal journal).
#[tauri::command]
pub async fn disable_online(
    app: tauri::AppHandle,
    app_id: u32,
    confirm_cleanup: Option<bool>,
) -> Result<OnlineActionResult, String> {
    let confirm_cleanup = confirm_cleanup.unwrap_or(false);
    let steam_root = crate::commands::command_steam_path(&app)?;
    let mutation =
        crate::core::game_mutations::acquire(Path::new(&steam_root), app_id, "disable_online")
            .await?;
    crate::core::game_mutations::ensure_current_root(&app, Path::new(&steam_root))?;
    let backup_root = LocalAppPaths::backup_root();
    let state_path = state_path();
    let game_root = resolve_installed_game(&app, app_id)
        .ok()
        .map(|game| PathBuf::from(game.game_path));

    crate::desk_log_info!("online", "disable_online: app={}", app_id);

    let (_mutation, result) = tauri::async_runtime::spawn_blocking(move || {
        let result = OnlineEngine::disable(
            app_id,
            &backup_root,
            &state_path,
            game_root.as_deref(),
            confirm_cleanup,
        );
        (mutation, result)
    })
    .await
    .map_err(|e| format!("Online worker failed: {e}"))?;
    let result = result?;

    if result.success {
        // UCO2 removed: the app leaves exclude_apps and returns to the global
        // default behavior — Show Online when default_mode=showonline
        // ("show online games" on), None otherwise. Without this undo the game
        // would stay excluded forever after deactivation.
        for path in aethercore_toml_paths(&app) {
            update_mode_in_toml(&path, app_id, None).map_err(|e| {
                format!("Online files reverted, presence configuration incomplete: {e}")
            })?;
        }
    }

    crate::desk_log_info!("online", "disable_online done: success={}", result.success);
    Ok(result)
}

/// Removes the online-fix.me crack files AND the UCOnline2 files in one go.
///
/// Triggered by the "Clean" button in the OFME banner: an online-fix.me drop
/// overwrites the UCO2 files, so both stacks are wiped and the folder is left
/// ready for a fresh Online choice. Without `confirm` the call only asks.
#[tauri::command]
pub async fn clean_online_conflicts(
    app: tauri::AppHandle,
    app_id: u32,
    confirm: Option<bool>,
) -> Result<OnlineActionResult, String> {
    if !confirm.unwrap_or(false) {
        return Ok(OnlineActionResult {
            success: false,
            message: crate::online::revert::CONFIRM_CLEAN_MESSAGE.into(),
            record: None,
            confirmation_required: true,
            warning: None,
        });
    }

    let steam_root = crate::commands::command_steam_path(&app)?;
    let mutation = crate::core::game_mutations::acquire(
        Path::new(&steam_root),
        app_id,
        "clean_online_conflicts",
    )
    .await?;
    crate::core::game_mutations::ensure_current_root(&app, Path::new(&steam_root))?;
    let backup_root = LocalAppPaths::backup_root();
    let state_path = state_path();
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);

    crate::desk_log_info!("online", "clean_online_conflicts: app={}", app_id);

    let (_mutation, result) = tauri::async_runtime::spawn_blocking(move || {
        let result = crate::online::revert::clean_online_conflicts(
            app_id,
            &backup_root,
            &state_path,
            &game_root,
        );
        (mutation, result)
    })
    .await
    .map_err(|e| format!("Online worker failed: {e}"))?;
    let (message, warning) = result?;

    // UCO2 is gone: return the game to the default presence behavior.
    for path in aethercore_toml_paths(&app) {
        update_mode_in_toml(&path, app_id, None)
            .map_err(|e| format!("Files removed, presence configuration incomplete: {e}"))?;
    }

    crate::desk_log_info!("online", "clean_online_conflicts done: app={}", app_id);
    Ok(OnlineActionResult {
        success: true,
        message,
        record: None,
        confirmation_required: false,
        warning,
    })
}

/// Locates the UCOnline2 bundle (installed or vendored).
///
/// Returns a short, user-facing error when the bundle is missing.
fn locate_bundle(app: &tauri::AppHandle) -> Result<Uco2Bundle, String> {
    let locator = ToolBundleLocator::new(
        app.clone(),
        UCO2_RESOURCE_DIR,
        LocalAppPaths::uco2_dir(),
        Uco2Bundle::is_valid_dir,
    );
    match locator.locate() {
        Ok(bundle) => Uco2Bundle::open(bundle.dir),
        Err(_) => Err("UCOnline2 bundle not found.".to_string()),
    }
}

/// True when a running process has the same file name as `exe`.
fn is_exe_running(exe_path: &Path) -> bool {
    let Some(target) = exe_path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let target_lower = target.to_ascii_lowercase();

    let mut system = sysinfo::System::new_all();
    // sysinfo 0.30: refresh_processes() takes no arguments
    // (the ProcessesToUpdate API arrived in 0.31).
    system.refresh_processes();
    system
        .processes()
        .values()
        .any(|process| process.name().to_ascii_lowercase() == target_lower)
}
