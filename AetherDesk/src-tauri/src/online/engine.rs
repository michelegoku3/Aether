//! UCOnline2 engine facade: plan, enable, disable, state.
//!
//! Composes detection + bundle + deploy/revert + state into a single pure API
//! (no Tauri dependency), used by the commands in `commands/online.rs`.

use crate::online::bundle::Uco2Bundle;
use crate::online::deploy;
use crate::online::detect::{overlay_target, GameInspector};
use crate::online::revert;
use crate::online::state::OnlineStateStore;
use crate::online::types::{
    Conflict, DetectionReport, OnlineActionResult, OnlineEnableRequest, OnlinePlan,
    OnlineStateKind, OnlineStatus, PhotonFlavor, Prerequisites,
};
use std::path::Path;

pub struct OnlineEngine;

impl OnlineEngine {
    /// Activation plan: fresh detection + prerequisites + current state.
    /// No disk effects.
    pub fn plan(
        app_id: u32,
        game_root: &Path,
        bundle: &Result<Uco2Bundle, String>,
        state_path: &Path,
    ) -> Result<OnlinePlan, String> {
        let detection = inspect(game_root)?;

        let (bundle_ok, bundle_version) = match bundle {
            Ok(bundle) => (true, bundle.version()),
            Err(_) => (false, None),
        };
        let steam_api_dir_writable = detection
            .steam_api_dir
            .as_deref()
            .map(probe_writable)
            .unwrap_or(false);

        let mut errors = Vec::new();
        if let Err(bundle_error) = bundle {
            errors.push(bundle_error.clone());
        }
        if detection.steam_api_dir.is_none() {
            errors
                .push("No place found for steam_api(64).dll (engine not recognized).".to_string());
        } else if !steam_api_dir_writable {
            errors.push(
                "The game folder is not writable. Run AetherDesk as administrator.".to_string(),
            );
        }

        let store = OnlineStateStore::load(state_path);
        let current = store.get(app_id).cloned();

        let notices = build_notices(&detection, bundle.as_ref().ok(), current.as_ref());

        Ok(OnlinePlan {
            detection,
            prerequisites: Prerequisites {
                bundle_ok,
                bundle_version,
                steam_api_dir_writable,
                errors,
            },
            current,
            notices,
        })
    }

    /// Enable UCOnline2 on a game (transactional, idempotent deploy).
    pub fn enable(
        app_id: u32,
        game_root: &Path,
        bundle: &Uco2Bundle,
        request: &OnlineEnableRequest,
        backup_root: &Path,
        state_path: &Path,
    ) -> Result<OnlineActionResult, String> {
        let detection = inspect(game_root)?;
        let record = deploy::deploy(app_id, &detection, bundle, request, backup_root, state_path)?;

        Ok(OnlineActionResult {
            success: true,
            message: format!(
                "Online enabled for app {} (ogAppId={}, spoof={}, backends: {}).",
                app_id,
                record.og_app_id,
                record.spoof_app_id,
                if record.backends_deployed.is_empty() {
                    "none".to_string()
                } else {
                    record.backends_deployed.join(", ")
                }
            ),
            record: Some(record),
            confirmation_required: false,
            warning: None,
        })
    }

    /// Disattiva UCOnline2 (rollback dal journal, idempotente).
    ///
    /// `confirm_cleanup` authorizes removing known UCO2 files when the recovery
    /// journal is gone; without it the call reports `confirmation_required`.
    pub fn disable(
        app_id: u32,
        backup_root: &Path,
        state_path: &Path,
        game_root: Option<&Path>,
        confirm_cleanup: bool,
    ) -> Result<OnlineActionResult, String> {
        match revert::disable(app_id, backup_root, state_path, game_root, confirm_cleanup)? {
            revert::DisableOutcome::ConfirmationRequired { message } => Ok(OnlineActionResult {
                success: false,
                message,
                record: None,
                confirmation_required: true,
                warning: None,
            }),
            revert::DisableOutcome::Done { message, warning } => Ok(OnlineActionResult {
                success: true,
                message,
                record: None,
                confirmation_required: false,
                warning,
            }),
        }
    }

    /// Reconciled state (record + files on disk).
    pub fn status(app_id: u32, state_path: &Path) -> OnlineStatus {
        let store = OnlineStateStore::load(state_path);
        let record = store.get(app_id).cloned();
        let state = match &record {
            None => OnlineStateKind::NotConfigured,
            Some(record) => {
                if record.ini_path.is_file() && record.steam_api_path.is_file() {
                    OnlineStateKind::Enabled
                } else {
                    OnlineStateKind::Broken
                }
            }
        };
        OnlineStatus { state, record }
    }
}

fn inspect(game_root: &Path) -> Result<DetectionReport, String> {
    GameInspector::inspect(game_root).map_err(|error| error.to_string())
}

/// Notices shown in the UI as a single ⚠ group (no more separate 💡 hints:
/// suggestions duplicating warnings were merged here).
/// Merges detection warnings with operational notes, deduplicated.
fn build_notices(
    detection: &DetectionReport,
    bundle: Option<&Uco2Bundle>,
    current: Option<&crate::online::types::OnlineRecord>,
) -> Vec<String> {
    let mut notices = detection.warnings.clone();

    if crate::online::foreign::has_ofme(&detection.conflicts) {
        notices.push(
            "online-fix.me / SteamFix files are in this folder. UCO2 Enable is blocked \
             until they are removed — the two Spacewar stacks cannot share a process."
                .to_string(),
        );
    } else if detection
        .conflicts
        .iter()
        .any(|c| matches!(c, Conflict::ColdClientLoader(_)))
    {
        notices.push(
            "A competing Steam client loader (steamclient64.ini) was detected: \
             it will be moved into the AetherDesk backup if you enable UCO2."
                .to_string(),
        );
    }

    if detection.backends.eos {
        notices.push(
            "Without at least the ProductId, EOS_custom will NOT be installed (avoids \
             the game getting stuck on the EOS login)."
                .to_string(),
        );
    }
    if detection.backends.photon != PhotonFlavor::None {
        notices.push(
            "Photon detected: your Photon app GUIDs (Realtime/Fusion and Voice when \
             present) are required for Photon multiplayer if it doesn't work out of \
             the box."
                .to_string(),
        );
    }
    if detection.backends.playfab && detection.backends.coherence {
        notices.push(
            "PlayFab detected alongside coherence: leave TitleId empty unless testing \
             proves PlayFab is required."
                .to_string(),
        );
    } else if detection.backends.playfab {
        notices.push(
            "PlayFab is the matchmaking backend: everyone must use the same TitleId \
             (SHARED uses the community title 1D861F). Blank = game launches, no multiplayer."
                .to_string(),
        );
    }
    if detection.backends.coherence {
        notices.push(
            "coherence detected: a runtime key is required (your own project with the \
             schema uploaded, or the shared community project)."
                .to_string(),
        );
    }
    if detection.steamless_applied {
        notices.push(
            "Steamless already processed one of the game executables: no conflict with \
             UCOnline2."
                .to_string(),
        );
    }
    if detection.steamstub_detected && !detection.steamless_applied {
        notices.push(
            "SteamStub detected on the executable: enable GetStubbedLol (runtime hook) \
             if Steamless cannot unpack it."
                .to_string(),
        );
    }

    match overlay_target(detection) {
        Ok(_) => {
            if bundle.is_some_and(|item| item.overlay_proxy_dll().is_none()) {
                notices.push(
                    "Early overlay proxy is not in this UCOnline2 bundle: Shift+Tab may \
                     be missing on engines that init graphics before steam_api."
                        .to_string(),
                );
            }
        }
        Err(reason) if reason.contains("Phasmophobia") => {
            notices.push(reason.to_string());
        }
        Err(_) => {}
    }

    if let (Some(record), Some(active_bundle)) = (current, bundle) {
        if record.bundle_version.as_deref() != active_bundle.version().as_deref() {
            notices.push(format!(
                "Deployed UCO2 {} — bundle is {}. Enable again to update the files.",
                record.bundle_version.as_deref().unwrap_or("unknown"),
                active_bundle.version().as_deref().unwrap_or("unknown"),
            ));
        }
    }

    notices
}

/// Write probe: creates and removes a temp file in the directory.
///
/// When the directory does not exist yet (convention fallback for games
/// without a steam_api(64).dll), it walks up to the first existing ancestor
/// and checks writability there — deploy will create it with `create_dir_all`.
fn probe_writable(dir: &Path) -> bool {
    let mut candidate = dir.to_path_buf();
    loop {
        if candidate.is_dir() {
            let probe = candidate.join(".aetherdesk_write_probe");
            if std::fs::write(&probe, b"probe").is_ok() {
                let _ = std::fs::remove_file(&probe);
                return true;
            }
            return false;
        }
        match candidate.parent() {
            Some(parent) => candidate = parent.to_path_buf(),
            None => return false,
        }
    }
}
