//! `union-crax.ini` generation (UCOnline2 configuration).
//!
//! Typed builder, faithfully mirroring the sections written by `patch.bat`
//! (v1.19.8 + PlayFab SHARED from HEAD). Project rules:
//!   * the ini is ALWAYS written next to the running exe (`ini_dir`
//!     computed by detection) — an ini in Unreal game roots would be
//!     silently ignored;
//!   * backend credentials go in the ini ONLY when the user provided them
//!     (stub sections otherwise);
//!   * `UnlockAll=true` always + DLC harvest from `configs.app.ini`
//!     (Goldberg) or from the numbered folders in the game dir.

use crate::external_tools::fs::walk_files;
use crate::online::types::{
    CoherenceOptions, DetectionReport, OnlineEnableRequest, PhotonFlavor, PlayfabOptions,
};
use std::path::Path;

/// Runtime key of the shared community coherence project (patch.bat):
/// client-side identifier, publishable; availability not guaranteed.
pub const COHERENCE_SHARED_KEY: &str = "fce1ea692a854b50b9f945ef6aa17758";

/// Shared community PlayFab TitleId (patch.bat `SHARED`, NMS and others).
pub const PLAYFAB_SHARED_TITLE: &str = "1D861F";

/// Harvested DLC entry: (appid, name).
pub type DlcEntry = (String, String);

/// Build the `union-crax.ini` content for a detected game.
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
    push_line(
        &mut out,
        &format!("VerboseLog={}", bool_str(request.verbose_log)),
    );
    push_line(
        &mut out,
        &format!("EmulateTicket={}", bool_str(request.emulate_ticket)),
    );
    push_line(
        &mut out,
        &format!(
            "WarnOverlayDisabled={}",
            bool_str(request.warn_overlay_disabled)
        ),
    );
    push_line(&mut out, &format!("SDR={}", bool_str(request.sdr)));
    push_line(
        &mut out,
        &format!("LoadOverlay={}", bool_str(request.load_overlay)),
    );
    push_line(
        &mut out,
        &format!("LogOverlay={}", bool_str(request.log_overlay)),
    );
    push_line(
        &mut out,
        &format!("GetStubbedLol={}", bool_str(request.get_stubbed_lol)),
    );
    let client = normalize_client(&request.client);
    if !client.is_empty() {
        push_line(&mut out, &format!("Client={client}"));
    }

    // [DLC] — UnlockAll (UI toggle) + harvested entries for games
    // that ENUMERATE DLC via GetDLCCount/BGetDLCDataByIndex.
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
    push_line(
        &mut out,
        &format!("UnlockAll={}", bool_str(request.unlock_all_dlc)),
    );
    for (id, name) in dlc_entries {
        push_line(&mut out, &format!("{id}={name}"));
    }

    // Photon — only when the plugin was requested (deploy_photon).
    // Realtime or Fusion (never both — patch.bat picks the flavor).
    if request.deploy_photon {
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
    }

    // EOS — only when the plugin was requested (deploy_eos_custom).
    if detection.backends.eos && request.deploy_eos_custom {
        push_line(&mut out, "");
        push_line(&mut out, "[EOS]");
        push_line(&mut out, &format!("ProductId={}", request.eos.product_id));
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
        push_line(
            &mut out,
            &format!("TitleId={}", playfab_title(&request.playfab)),
        );
    }

    out
}

/// Final coherence runtime key: the user's own key, or the shared community
/// project's key when `use_shared` is on.
pub fn coherence_key(options: &CoherenceOptions) -> String {
    if options.use_shared {
        COHERENCE_SHARED_KEY.to_string()
    } else {
        options.runtime_key.clone()
    }
}

/// TitleId PlayFab finale: propria, `SHARED` digitato, oppure il title community.
pub fn playfab_title(options: &PlayfabOptions) -> String {
    if options.use_shared || options.title_id.trim().eq_ignore_ascii_case("SHARED") {
        PLAYFAB_SHARED_TITLE.to_string()
    } else {
        options.title_id.clone()
    }
}

/// Accept "017" or "SteamClient017"; return the short form to write
/// in the ini (UCO2 accepts both). Empty when the user pinned nothing.
pub fn normalize_client(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let stripped = trimmed
        .strip_prefix("SteamClient")
        .or_else(|| trimmed.strip_prefix("steamclient"))
        .unwrap_or(trimmed)
        .trim();
    stripped.to_string()
}

/// Harvest DLC from the game, from the same two patch.bat sources:
///   1. first `configs.app.ini` found (Goldberg) — `appid=name` lines;
///   2. otherwise the numerically-named folders in `ini_dir` — `id=DLC id`.
pub fn harvest_dlc(game_root: &Path, ini_dir: &Path) -> Vec<DlcEntry> {
    if let Some(entries) = harvest_from_configs_app_ini(game_root) {
        return entries;
    }
    harvest_from_numeric_dirs(ini_dir)
}

fn harvest_from_configs_app_ini(game_root: &Path) -> Option<Vec<DlcEntry>> {
    let configs = walk_files(game_root).into_iter().find(|path| {
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

fn bool_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}
