//! Shared Steam publication pipeline for provider packages.
//!
//! Provider acquisition is deliberately absent from this module. Callers pass
//! a complete package; this module owns mutation locking, ordered publication,
//! completeness verification, update policy, backup and library notification.

use super::package_completion::enabled_pins_from_content;
use crate::core::backup::GameBackup;
use crate::core::game_mutations::MutationPlan;
use crate::core::settings::load_settings;
use crate::manifest::package::{ManifestPackage, ManifestPackageFile};
use crate::manifest::pins::{LuaManifestPins, LuaManifestRow};
use crate::manifest::resolver;
use crate::steam::compat::SteamCompat;

#[derive(Debug, Clone, Copy)]
pub(super) struct SpecificInstallPolicy {
    /// Retains the historical Hubcap-specific behavior while allowing Ryuu and
    /// LuaTools switches to clear partial Steam downloads before publication.
    pub clear_residual_download: bool,
}

impl Default for SpecificInstallPolicy {
    fn default() -> Self {
        Self {
            clear_residual_download: true,
        }
    }
}

pub(super) async fn install_standard_package(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    package: ManifestPackage,
    source: &str,
    plan: MutationPlan,
) -> Result<String, String> {
    let mutation = plan.commit_for(app).await?;
    let steam = SteamCompat::new(steam_path);
    steam.clear_residual_download_state(app_id);
    steam.install_lua_and_manifest_files(app_id, &package.lua_content, &package.manifest_files)?;
    restore_local_manifests(app_id, steam_path, &package.lua_content).await?;
    verify_referenced_manifests(app_id, steam_path, &package.lua_content)?;
    apply_update_policy_and_backup(
        app,
        app_id,
        steam_path,
        &package.lua_content,
        &package.manifest_files,
    )?;

    // The refresh acquires its own mutation plan; never nest per-game locks.
    drop(mutation);
    match crate::manifest::pin_refresh::refresh_game_pins_from_hubcap(app.clone(), app_id).await {
        Ok(report) if report.staged > 0 || report.realigned > 0 => crate::desk_log_info!(
            "store",
            "Post-install pin refresh app_id={} staged={} realigned={}",
            app_id,
            report.staged,
            report.realigned
        ),
        Ok(_) => {}
        Err(error) => crate::desk_log_warn!(
            "store",
            "Post-install pin refresh not applied app_id={}: {}",
            app_id,
            error
        ),
    }
    notify_store_change(app, app_id);
    Ok(format!(
        "Successfully completed {} download for App ID {}. Lua installed, {} manifest file(s) preloaded into Steam depotcache.",
        source,
        app_id,
        package.manifest_files.len()
    ))
}

pub(super) async fn install_specific_package(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    package: ManifestPackage,
    source: &str,
    plan: MutationPlan,
    policy: SpecificInstallPolicy,
) -> Result<Vec<LuaManifestRow>, String> {
    let manifest_rows = LuaManifestPins::rows_from_content(&package.lua_content);
    if manifest_rows.is_empty() {
        return Err(format!(
            "The downloaded Lua from {} does not contain any setManifestid entries, so it was not installed.",
            source
        ));
    }

    let _mutation = plan.commit_for(app).await?;
    let steam = SteamCompat::new(steam_path);
    if policy.clear_residual_download {
        steam.clear_residual_download_state(app_id);
    }
    steam.install_lua_and_manifest_files(app_id, &package.lua_content, &package.manifest_files)?;
    restore_local_manifests(app_id, steam_path, &package.lua_content).await?;
    verify_referenced_manifests(app_id, steam_path, &package.lua_content)?;
    apply_update_policy_and_backup(
        app,
        app_id,
        steam_path,
        &package.lua_content,
        &package.manifest_files,
    )?;

    let installed_rows = LuaManifestPins::new(steam_path.to_string(), app_id).rows_from_file()?;
    if installed_rows.len() != manifest_rows.len() {
        return Err(format!(
            "Lua install verification failed: downloaded file had {} setManifestid entries, installed file has {}.",
            manifest_rows.len(),
            installed_rows.len()
        ));
    }
    notify_store_change(app, app_id);
    Ok(installed_rows)
}

async fn restore_local_manifests(
    app_id: u32,
    steam_path: &str,
    lua_content: &str,
) -> Result<(), String> {
    resolver::resolve(resolver::ManifestRequest {
        steam_path: steam_path.to_string(),
        app_id,
        pins: enabled_pins_from_content(lua_content),
        generation: resolver::Generation::LocalOnly,
    })
    .await?;
    Ok(())
}

fn verify_referenced_manifests(
    app_id: u32,
    steam_path: &str,
    lua_content: &str,
) -> Result<(), String> {
    let pins = enabled_pins_from_content(lua_content);
    let completeness = resolver::verify_available(steam_path, app_id, &pins);
    if !completeness.missing.is_empty() {
        let missing = completeness
            .missing
            .iter()
            .map(|pin| format!("{}_{}", pin.depot_id, pin.manifest_id))
            .collect::<Vec<_>>()
            .join(", ");
        crate::desk_log_error!(
            "store",
            "Manifest completion gate failed app_id={} missing={}",
            app_id,
            completeness.missing.len()
        );
        return Err(format!(
            "Package was not completed: {} referenced manifest(s) are missing or empty after install: {}",
            completeness.missing.len(),
            missing
        ));
    }
    crate::desk_log_info!(
        "store",
        "Manifest completion gate passed app_id={} verified={}",
        app_id,
        completeness.verified
    );
    Ok(())
}

fn apply_update_policy_and_backup(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
    lua_fallback: &str,
    manifests: &[ManifestPackageFile],
) -> Result<(), String> {
    apply_default_update_policy(app, app_id, steam_path)?;
    let installed_lua = SteamCompat::new(steam_path)
        .read_lua_config(app_id)
        .unwrap_or_else(|_| lua_fallback.to_string());
    GameBackup::for_app(app_id)?.backup_lua_artifacts(app_id, &installed_lua, manifests)?;
    Ok(())
}

fn apply_default_update_policy(
    app: &tauri::AppHandle,
    app_id: u32,
    steam_path: &str,
) -> Result<(), String> {
    let settings = load_settings(app);
    crate::desk_log_debug!(
        "store",
        "Update policy after package install app_id={} enabled={}",
        app_id,
        settings.download_games_with_updates_on
    );
    if settings.download_games_with_updates_on {
        LuaManifestPins::new(steam_path.to_string(), app_id)
            .set_updates_enabled(true)
            .map(|_| ())?;
    }
    Ok(())
}

fn notify_store_change(app: &tauri::AppHandle, app_id: u32) {
    crate::core::library_events::notify_lua_changed(
        app,
        crate::core::library_events::LibraryChangeOrigin::Store,
        [app_id],
    );
}
