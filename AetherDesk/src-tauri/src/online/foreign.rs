//! Classificazione delle crack **esterne** e dei file UCO2 già sul disco.
//!
//! NON è AetherOnline (il payload di Aether, token `-aetheronline`): qui si
//! rilevano solo file TERZI di OFME (online-fix.me) e UCO2. Unico posto in
//! cui si decide “questi file sono OFME / UCO2”: detection, enable_online,
//! set_aether_* e la UI leggono da qui. La scansione è ricorsiva: Bodycam e
//! gli Unreal tengono i marker in `Binaries/Win64`, non accanto a
//! `steam_api`. Vocabolario completo: AGENTS.md in radice.

use crate::online::types::Conflict;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Nomi file che, da soli, identificano un pack online-fix.me / SteamFix.
const OFME_STRONG: &[&str] = &[
    "onlinefix64.dll",
    "onlinefix.dll",
    "onlinefix.ini",
    "onlinefix.json",
    "steamfix64.dll",
    "steamfix.dll",
    "steamfix.ini",
];

const OFME_SUPPORT: &[&str] = &[
    "onlinefix.url",
    "dlllist.txt",
    "steamoverlay64.dll",
    "stubdrm64.dll",
    "photonbridge.dll",
];

/// Marker UCO2 (non OFME). `union-crax.ini` è la firma; i plugin/proxy
/// hanno nomi propri della release UCOnline2.
const UCO2_MARKERS: &[&str] = &[
    "union-crax.ini",
    "overlay_proxy.dll",
    "photon_universal.dll",
    "eos_custom.dll",
    "coherence_universal.dll",
];

/// Cartelle Unreal/asset da non scansionare: niente crack lì, e Bodycam
/// ha decine di GB in `Content/`.
const SKIP_DIRS: &[&str] = &[
    "content",
    "intermediate",
    "saved",
    "deriveddatacache",
    ".git",
    ".svn",
    "__pycache__",
    "node_modules",
];

/// Esito serializzato verso la UI (camelCase).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignOnlineReport {
    /// Pack online-fix.me / SteamFix sul disco (anche innestato).
    pub ofme: bool,
    /// File UCOnline2 già presenti (anche senza record Aether).
    pub uco2: bool,
    /// Path OFME che hanno fatto scattare il verdetto (per i messaggi).
    pub files: Vec<PathBuf>,
}

impl ForeignOnlineReport {
    pub fn from_conflicts(conflicts: &[Conflict]) -> Self {
        let files = ofme_files(conflicts);
        Self {
            ofme: !files.is_empty(),
            uco2: false,
            files,
        }
    }

    pub fn refuse_uco2(&self) -> String {
        format!(
            "online-fix.me files are in this folder ({}). Remove them before enabling UCO2 — the two Spacewar stacks cannot share a process.",
            display_files(&self.files)
        )
    }

    pub fn refuse_online_aether(&self) -> String {
        format!(
            "online-fix.me files are in this folder ({}). Online Aether is Aether's own payload, not that crack. Keep None to use the crack, or remove the files to use Online Aether.",
            display_files(&self.files)
        )
    }

    pub fn refuse_showonline(&self) -> String {
        format!(
            "This folder already spoofs Spacewar via online-fix.me ({}). Show Online would remap presence again and break invites. Keep None.",
            display_files(&self.files)
        )
    }

    pub fn refuse_showonline_uco2(&self) -> String {
        "UCO2 files are already in this folder. Show Online remaps Spacewar and breaks invites. Keep None.".to_string()
    }

    pub fn refuse_online_aether_uco2(&self) -> String {
        "UCO2 files are already in this folder. Online Aether and UCO2 cannot share a process.".to_string()
    }
}

/// Scansione ricorsiva indipendente da Unity/Unreal: funziona anche se
/// `GameInspector` non riconosce il gioco.
pub fn scan(root: &Path) -> ForeignOnlineReport {
    let mut ofme = Vec::new();
    let mut uco2 = Vec::new();

    for path in walk_marker_files(root) {
        let Some(lower) = file_name_lower(&path) else {
            continue;
        };
        if UCO2_MARKERS.iter().any(|n| *n == lower) {
            uco2.push(path);
            continue;
        }
        if OFME_STRONG.iter().any(|n| *n == lower) || OFME_SUPPORT.iter().any(|n| *n == lower) {
            ofme.push(path);
            continue;
        }
        if is_winmm_name(&lower) && ofme_sibling(&path) {
            ofme.push(path);
        }
    }

    ForeignOnlineReport {
        ofme: !ofme.is_empty(),
        uco2: !uco2.is_empty(),
        files: ofme,
    }
}

/// Conflitto da quarantinare per un file OFME già classificato.
pub fn conflict_for_ofme_file(path: &Path) -> Conflict {
    let lower = file_name_lower(path).unwrap_or_default();
    match lower.as_str() {
        "steamfix64.dll" | "steamfix.dll" | "steamfix.ini" => Conflict::SteamFix(path.to_path_buf()),
        "onlinefix64.dll" | "onlinefix.dll" => Conflict::OFME(path.to_path_buf()),
        _ => Conflict::NamedFixFile(path.to_path_buf()),
    }
}

/// File OFME/SteamFix da una lista di conflitti già calcolata.
pub fn ofme_files(conflicts: &[Conflict]) -> Vec<PathBuf> {
    conflicts
        .iter()
        .filter_map(|conflict| match conflict {
            Conflict::OFME(path) | Conflict::SteamFix(path) => Some(path.clone()),
            Conflict::NamedFixFile(path) if named_is_ofme(path) => Some(path.clone()),
            _ => None,
        })
        .collect()
}

pub fn has_ofme(conflicts: &[Conflict]) -> bool {
    !ofme_files(conflicts).is_empty()
}

fn named_is_ofme(path: &Path) -> bool {
    let Some(lower) = file_name_lower(path) else {
        return false;
    };
    if OFME_STRONG.iter().any(|n| *n == lower) || OFME_SUPPORT.iter().any(|n| *n == lower) {
        return true;
    }
    is_winmm_name(&lower) && ofme_sibling(path)
}

fn is_winmm_name(lower: &str) -> bool {
    lower == "winmm.dll" || lower == "winmm.ini" || lower == "winmm.txt"
}

fn ofme_sibling(path: &Path) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    OFME_STRONG
        .iter()
        .chain(OFME_SUPPORT.iter())
        .any(|name| dir.join(name).is_file())
}

fn file_name_lower(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_ascii_lowercase())
}

fn walk_marker_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let skip = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| SKIP_DIRS.iter().any(|s| n.eq_ignore_ascii_case(s)))
                    .unwrap_or(false);
                if !skip {
                    stack.push(path);
                }
            } else if kind.is_file() {
                out.push(path);
            }
        }
    }
    out
}

/// True quando il file è un artefatto UCO2 (mai da ripristinare come originale).
pub fn is_uco2_owned_name(path: &Path) -> bool {
    file_name_lower(path)
        .map(|lower| UCO2_MARKERS.iter().any(|n| *n == lower))
        .unwrap_or(false)
}

/// Rimuove i marker UCO2 dal disco. Non tocca steam_api(64).dll (senza
/// backup l'originale non è recuperabile). Best-effort: non fallisce.
pub fn sweep_uco2_files(root: &Path) -> usize {
    let mut removed = 0;
    for path in walk_marker_files(root) {
        if !is_uco2_owned_name(&path) {
            continue;
        }
        if path.is_file() && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn display_files(files: &[PathBuf]) -> String {
    files
        .iter()
        .filter_map(|p| p.file_name()?.to_str())
        .collect::<Vec<_>>()
        .join(", ")
}
