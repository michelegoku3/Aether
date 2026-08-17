//! Generazione di `union-crax.ini` (configurazione UCOnline2).
//!
//! Builder tipizzato, specchio fedele delle sezioni che scrive `patch.bat`
//! (commit 797d550). Regole di progetto:
//!   * l'ini si scrive SEMPRE accanto all'exe in esecuzione (`ini_dir`
//!     calcolato dalla detection) — un ini nella root dei giochi Unreal
//!     verrebbe ignorato in silenzio;
//!   * le credenziali backend vanno nell'ini SOLO se l'utente le ha
//!     fornite (sezioni stub altrimenti);
//!   * `UnlockAll=true` sempre + harvest DLC da `configs.app.ini`
//!     (Goldberg) o dalle cartelle numeriche nella dir di gioco.

use crate::online::types::{
    CoherenceOptions, DetectionReport, OnlineEnableRequest, PhotonFlavor,
};
use crate::external_tools::fs::walk_files;
use std::path::Path;

/// Runtime key del progetto coherence community condiviso (patch.bat):
/// client-side identifier, pubblicabile; disponibilità non garantita.
pub const COHERENCE_SHARED_KEY: &str = "fce1ea692a854b50b9f945ef6aa17758";

/// Entry DLC harvestata: (appid, nome).
pub type DlcEntry = (String, String);

/// Costruisce il contenuto di `union-crax.ini` per un gioco rilevato.
pub fn build_ini(
    detection: &DetectionReport,
    request: &OnlineEnableRequest,
    dlc_entries: &[DlcEntry],
) -> String {
    let mut out = String::new();
    push_line(&mut out, "[Settings]");
    push_line(&mut out, &format!("AppId={}", request.spoof_app_id));
    push_line(&mut out, &format!("ogAppId={}", request.og_app_id));
    push_line(&mut out, "PluginsFolder=plugins");

    // [DLC] — sempre UnlockAll=true; le entry harvestate servono ai giochi
    // che ENUMERANO i DLC via GetDLCCount/BGetDLCDataByIndex.
    push_line(&mut out, "");
    push_line(&mut out, "[DLC]");
    push_line(
        &mut out,
        "; UnlockAll answers any \"do I own this DLC?\" check, for any id, so DLC",
    );
    push_line(&mut out, "; works without knowing what the ids are.");
    push_line(
        &mut out,
        "; The \"appid=name\" lines below are what a game reads when it ENUMERATES",
    );
    push_line(
        &mut out,
        "; its DLC to build a menu. Both work together - UnlockAll is the fallback.",
    );
    push_line(&mut out, "UnlockAll=true");
    for (id, name) in dlc_entries {
        push_line(&mut out, &format!("{id}={name}"));
    }

    // Photon: Realtime o Fusion (mai entrambi — patch.bat sceglie il flavor).
    match detection.backends.photon {
        PhotonFlavor::None => {}
        PhotonFlavor::Fusion => {
            push_line(&mut out, "");
            push_line(&mut out, "[Fusion]");
            push_line(
                &mut out,
                &format!("PhotonAppIdFusion={}", request.photon.fusion_guid),
            );
            push_line(&mut out, "ForcedAuthType=0");
        }
        PhotonFlavor::Realtime => {
            push_line(&mut out, "");
            push_line(&mut out, "[Realtime]");
            push_line(
                &mut out,
                &format!("PhotonAppIdRealtime={}", request.photon.realtime_guid),
            );
            if detection.backends.photon_voice {
                push_line(
                    &mut out,
                    &format!("PhotonAppIdVoice={}", request.photon.voice_guid),
                );
            }
            push_line(&mut out, "ForcedAuthType=0");
        }
    }

    // EOS — solo se il plugin è stato richiesto (deploy_eos_custom).
    if detection.backends.eos && request.deploy_eos_custom {
        push_line(&mut out, "");
        push_line(&mut out, "[EOS]");
        push_line(
            &mut out,
            &format!("ProductId={}", request.eos.product_id),
        );
        push_line(&mut out, &format!("SandboxId={}", request.eos.sandbox_id));
        push_line(
            &mut out,
            &format!("DeploymentId={}", request.eos.deployment_id),
        );
        push_line(&mut out, &format!("ClientId={}", request.eos.client_id));
        push_line(
            &mut out,
            &format!("ClientSecret={}", request.eos.client_secret),
        );
        push_line(&mut out, "DisplayName=Player");
    }

    // coherence — ForceGuestLogin + runtime key (propria o SHARED).
    if detection.backends.coherence {
        push_line(&mut out, "");
        push_line(&mut out, "[Coherence]");
        push_line(&mut out, "ForceGuestLogin=true");
        let key = coherence_key(&request.coherence);
        push_line(&mut out, &format!("RuntimeKey={key}"));
        push_line(&mut out, "LocalMode=false");
    }

    // PlayFab.
    if detection.backends.playfab {
        push_line(&mut out, "");
        push_line(&mut out, "[PlayFab]");
        push_line(&mut out, &format!("TitleId={}", request.playfab.title_id));
    }

    out
}

/// Runtime key coherence finale: la chiave propria, oppure quella del
/// progetto community condiviso quando `use_shared` è attivo.
pub fn coherence_key(options: &CoherenceOptions) -> String {
    if options.use_shared {
        COHERENCE_SHARED_KEY.to_string()
    } else {
        options.runtime_key.clone()
    }
}

/// Harvest DLC dal gioco, con le stesse due sorgenti di patch.bat:
///   1. primo `configs.app.ini` trovato (Goldberg) — righe `appid=name`;
///   2. altrimenti le cartelle con nome numerico in `ini_dir` — `id=DLC id`.
pub fn harvest_dlc(game_root: &Path, ini_dir: &Path) -> Vec<DlcEntry> {
    if let Some(entries) = harvest_from_configs_app_ini(game_root) {
        return entries;
    }
    harvest_from_numeric_dirs(ini_dir)
}

fn harvest_from_configs_app_ini(game_root: &Path) -> Option<Vec<DlcEntry>> {
    let configs = walk_files(game_root)
        .into_iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("configs.app.ini"))
                .unwrap_or(false)
        })?;

    let content = std::fs::read_to_string(&configs).ok()?;
    let entries: Vec<DlcEntry> = content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (id, name) = line.split_once('=')?;
            if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() {
                Some((id.to_string(), name.trim().to_string()))
            } else {
                None
            }
        })
        .collect();

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

fn harvest_from_numeric_dirs(ini_dir: &Path) -> Vec<DlcEntry> {
    let Ok(entries) = std::fs::read_dir(ini_dir) else {
        return Vec::new();
    };
    let mut dlc: Vec<DlcEntry> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && !name.is_empty()
                && name.chars().all(|c| c.is_ascii_digit())
            {
                Some((name.clone(), format!("DLC {name}")))
            } else {
                None
            }
        })
        .collect();
    dlc.sort();
    dlc
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push_str("\r\n");
}
