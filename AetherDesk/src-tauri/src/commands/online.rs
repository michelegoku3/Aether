//! Comandi Tauri per UCOnline2 (Enable Online).
//!
//! Thin layer: risolve il gioco installato, localizza il bundle e delega
//! all'engine puro (`online::engine`). Tutto il lavoro pesante è eseguito
//! su `spawn_blocking` (pattern di `commands/steamless.rs`).

use crate::external_tools::bundle::ToolBundleLocator;
use crate::online::bundle::Uco2Bundle;
use crate::online::engine::OnlineEngine;
use crate::online::types::{OnlineEnableRequest, OnlinePlan, OnlineStatus, OnlineActionResult};
use crate::util::game_resolver::resolve_installed_game;
use crate::core::paths::LocalAppPaths;
use std::path::{Path, PathBuf};

const UCO2_RESOURCE_DIR: &str = "ExternalTools/UCOnline2";

/// Stato online di un gioco (per la UI: chip + record).
#[tauri::command]
pub async fn get_online_status(app_id: u32) -> Result<OnlineStatus, String> {
    let state_path = LocalAppPaths::state_dir().join("uc_online2.json");

    Ok(OnlineEngine::status(app_id, &state_path))
}

/// Piano di attivazione (dry-run: nessun effetto sul disco).
#[tauri::command]
pub async fn plan_online(app: tauri::AppHandle, app_id: u32) -> Result<OnlinePlan, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);

    let bundle = locate_bundle(&app);
    let state_path = LocalAppPaths::state_dir().join("uc_online2.json");

    let mut plan = OnlineEngine::plan(app_id, &game_root, &bundle, &state_path)?;

    // Verifica "gioco in esecuzione" (solo informativa nel piano).
    if let Some(exe) = &plan.detection.game_exe {
        if is_exe_running(exe) {
            plan.suggestions.push(format!(
                "Il gioco sembra essere in esecuzione ({}): chiudilo prima di attivare.",
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

/// Attiva UCOnline2 su un gioco (deploy transazionale).
#[tauri::command]
pub async fn enable_online(
    app: tauri::AppHandle,
    app_id: u32,
    request: OnlineEnableRequest,
) -> Result<OnlineActionResult, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);

    // Pre-flight: niente gioco in esecuzione durante il deploy.
    let inspection = OnlineEngine::plan(
        app_id,
        &game_root,
        &Err("bundle not needed for inspection".to_string()),
        &LocalAppPaths::state_dir().join("uc_online2.json"),
    )?;
    if let Some(exe) = &inspection.detection.game_exe {
        if is_exe_running(exe) {
            return Err(format!(
                "Close the game before enabling online ({}).",
                exe.file_name().and_then(|n| n.to_str()).unwrap_or("exe")
            ));
        }
    }

    let bundle = locate_bundle(&app)?;
    let backup_root = LocalAppPaths::backup_root();
    let state_path = LocalAppPaths::state_dir().join("uc_online2.json");

    crate::desk_log_info!(
        "online",
        "enable_online: app={} ogAppId={} spoof={} stub={} eos_deploy={}",
        app_id,
        request.og_app_id,
        request.spoof_app_id,
        request.steam_stub_patch,
        request.deploy_eos_custom
    );

    let result = tauri::async_runtime::spawn_blocking(move || {
        OnlineEngine::enable(app_id, &game_root, &bundle, &request, &backup_root, &state_path)
    })
    .await
    .map_err(|e| format!("Online worker failed: {e}"))??;

    crate::desk_log_info!("online", "enable_online done: success={} message='{}'", result.success, result.message);
    Ok(result)
}

/// Disattiva UCOnline2 (rollback dal journal).
#[tauri::command]
pub async fn disable_online(app_id: u32) -> Result<OnlineActionResult, String> {
    let backup_root = LocalAppPaths::backup_root();
    let state_path = LocalAppPaths::state_dir().join("uc_online2.json");

    crate::desk_log_info!("online", "disable_online: app={}", app_id);

    let result = tauri::async_runtime::spawn_blocking(move || {
        OnlineEngine::disable(app_id, &backup_root, &state_path)
    })
    .await
    .map_err(|e| format!("Online worker failed: {e}"))??;

    crate::desk_log_info!("online", "disable_online done: success={}", result.success);
    Ok(result)
}

/// Localizza il bundle UCOnline2 (installato o vendored).
///
/// In caso di errore restituisce un messaggio con i path ESATTI controllati,
/// così l'utente sa dove mettere la cartella (build portabile e repo).
fn locate_bundle(app: &tauri::AppHandle) -> Result<Uco2Bundle, String> {
    let locator = ToolBundleLocator::new(
        app.clone(),
        UCO2_RESOURCE_DIR,
        LocalAppPaths::uco2_dir(),
        Uco2Bundle::is_valid_dir,
    );
    match locator.locate() {
        Ok(bundle) => Uco2Bundle::open(bundle.dir),
        Err(_) => Err(format!(
            "Bundle UCOnline2 non trovato. Cercato in:\n  - {} (installato)\n  - {} (bundled nella repo)\nCopiaci la cartella 'UCOnline2' (con x86/, x64/ e plugins/) oppure scarica l'ultima release da https://github.com/LukeWarmSodas/uc-online2/releases ed estraila come ExternalTools\\UCOnline2.",
            LocalAppPaths::uco2_dir().display(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(UCO2_RESOURCE_DIR).display()
        )),
    }
}

/// True quando un processo in esecuzione ha lo stesso nome file di `exe`.
fn is_exe_running(exe_path: &Path) -> bool {
    let Some(target) = exe_path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let target_lower = target.to_ascii_lowercase();

    let mut system = sysinfo::System::new_all();
    // sysinfo 0.30: refresh_processes() non accetta argomenti
    // (l'API con ProcessesToUpdate è arrivata in 0.31).
    system.refresh_processes();
    system
        .processes()
        .values()
        .any(|process| process.name().to_ascii_lowercase() == target_lower)
}
