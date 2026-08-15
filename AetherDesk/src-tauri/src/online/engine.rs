//! Facade dell'engine UCOnline2: piano, abilitazione, disattivazione, stato.
//!
//! Compone detection + bundle + deploy/revert + stato in un'unica API
//! pura (nessuna dipendenza Tauri), usata dai comandi in
//! `commands/online.rs`.

use crate::online::bundle::Uco2Bundle;
use crate::online::deploy;
use crate::online::detect::GameInspector;
use crate::online::revert;
use crate::online::state::OnlineStateStore;
use crate::online::types::{
    Conflict, DetectionReport, OnlineActionResult, OnlineEnableRequest, OnlinePlan,
    OnlineStateKind, OnlineStatus, PhotonFlavor, Prerequisites,
};
use std::path::Path;

pub struct OnlineEngine;

impl OnlineEngine {
    /// Piano di attivazione: detection fresca + prerequisiti + stato attuale.
    /// Nessun effetto sul disco.
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
            errors.push(
                "No place found for steam_api(64).dll (engine not recognized)."
                    .to_string(),
            );
        } else if !steam_api_dir_writable {
            errors.push(
                "The game folder is not writable. Run AetherDesk as administrator."
                    .to_string(),
            );
        }

        let store = OnlineStateStore::load(state_path);
        let current = store.get(app_id).cloned();

        let suggestions = build_suggestions(&detection);

        Ok(OnlinePlan {
            detection,
            prerequisites: Prerequisites {
                bundle_ok,
                bundle_version,
                steam_api_dir_writable,
                errors,
            },
            current,
            suggestions,
        })
    }

    /// Attiva UCOnline2 su un gioco (deploy transazionale, idempotente).
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
        })
    }

    /// Disattiva UCOnline2 (rollback dal journal, idempotente).
    pub fn disable(
        app_id: u32,
        backup_root: &Path,
        state_path: &Path,
    ) -> Result<OnlineActionResult, String> {
        revert::disable(app_id, backup_root, state_path)?;
        Ok(OnlineActionResult {
            success: true,
            message: "Online disabled: files restored and state cleared.".to_string(),
            record: None,
        })
    }

    /// Stato riconciliato (record + file sul disco).
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

/// Suggerimenti fattuali per la UI (mai blocchi: decide l'utente).
fn build_suggestions(detection: &DetectionReport) -> Vec<String> {
    let mut suggestions = Vec::new();

    if detection.conflicts.iter().any(|c| {
        matches!(
            c,
            Conflict::ColdClientLoader(_)
                | Conflict::SteamFix(_)
                | Conflict::OnlineFix(_)
                | Conflict::NamedFixFile(_)
        )
    }) {
        suggestions.push(
            "Rilevato un emulatore concorrente (ColdClientLoader/SteamFix/OnlineFix): \
             verrà neutralizzato in modo reversibile (*.uco-disabled)."
                .to_string(),
        );
    }

    if detection.backends.eos {
        suggestions.push(
            "EOS rilevato: fornisci le credenziali di un'app Epic tua per deployare \
             EOS_custom (o lascia vuoto per saltare)."
                .to_string(),
        );
    }
    if detection.backends.photon != PhotonFlavor::None {
        suggestions.push(
            "Photon rilevato: servono i GUID degli app Photon (Realtime/Fusion e Voice \
             se presente) per il multiplayer Photon."
                .to_string(),
        );
    }
    if detection.backends.playfab {
        suggestions.push(
            "PlayFab rilevato: il plugin resta inerte finché non imposti un TitleId."
                .to_string(),
        );
    }
    if detection.backends.coherence {
        suggestions.push(
            "coherence rilevato: serve una runtime key (progetto tuo con schema caricato, \
             oppure il progetto SHARED community)."
                .to_string(),
        );
    }
    if !detection.backends.has_any() {
        suggestions.push(
            "Nessun backend secondario: se il multiplayer è Steam P2P puro, prova prima \
             senza plugin."
                .to_string(),
        );
    }
    if detection.steamless_applied {
        suggestions.push(
            "Steamless ha già processato un exe del gioco: nessun conflitto con UCOnline2."
                .to_string(),
        );
    }

    suggestions
}

/// Probe di scrittura: crea e rimuove un file temporaneo nella directory.
///
/// Se la directory non esiste ancora (fallback convenzione per i giochi che
/// non hanno una steam_api(64).dll), risale al primo antenato esistente e
/// verifica la scrivibilità lì — il deploy la creerà con `create_dir_all`.
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
