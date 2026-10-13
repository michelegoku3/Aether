//! Classification of **foreign** cracks and UCO2 files already on disk.
//!
//! This is NOT AetherOnline (the Aether payload, `-aetheronline` token): only
//! THIRD-PARTY OFME (online-fix.me) and UCO2 files are detected here. The single
//! place that decides "these files are OFME / UCO2": detection, enable_online,
//! set_aether_* and the UI all read from here. Scanning is recursive: Bodycam and
//! Unreal titles keep markers in `Binaries/Win64`, not next to `steam_api`.
//! Full vocabulary: AGENTS.md at the root.

use crate::online::types::Conflict;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// File names that alone identify an online-fix.me / SteamFix pack.
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

/// UCO2 markers (not OFME). `union-crax.ini` is the signature; plugins/proxies
/// carry UCOnline2 release proper names.
const UCO2_MARKERS: &[&str] = &[
    "union-crax.ini",
    "overlay_proxy.dll",
    "photon_universal.dll",
    "eos_custom.dll",
    "coherence_universal.dll",
];

/// Unreal/asset folders to skip while scanning: no cracks there, and Bodycam
/// holds tens of GB in `Content/`.
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

/// Outcome serialized to the UI (camelCase).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignOnlineReport {
    /// online-fix.me / SteamFix pack on disk (even nested).
    pub ofme: bool,
    /// UCOnline2 files already present (even with no Aether record).
    pub uco2: bool,
    /// OFME paths that triggered the verdict (for messages).
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
        "UCO2 files are already in this folder. Online Aether and UCO2 cannot share a process."
            .to_string()
    }
}

/// Family stems: any file name containing one (already lowercased) belongs
/// to an online-fix.me / SteamFix pack, including variant spellings the
/// exact lists above never heard of. Legit games do not ship such names,
/// so this is a strong signal, not a guess.
fn has_ofme_stem(lower: &str) -> bool {
    lower.contains("onlinefix") || lower.contains("steamfix")
}

/// Engine-independent recursive scan: works even when `GameInspector`
/// does not recognize the game.
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
        if OFME_STRONG.iter().any(|n| *n == lower)
            || OFME_SUPPORT.iter().any(|n| *n == lower)
            || has_ofme_stem(&lower)
        {
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

/// Conflict to quarantine for an already-classified OFME file.
pub fn conflict_for_ofme_file(path: &Path) -> Conflict {
    let lower = file_name_lower(path).unwrap_or_default();
    match lower.as_str() {
        "steamfix64.dll" | "steamfix.dll" | "steamfix.ini" => {
            Conflict::SteamFix(path.to_path_buf())
        }
        "onlinefix64.dll" | "onlinefix.dll" => Conflict::Ofme(path.to_path_buf()),
        _ => Conflict::NamedFixFile(path.to_path_buf()),
    }
}

/// OFME/SteamFix files from an already-computed conflict list.
pub fn ofme_files(conflicts: &[Conflict]) -> Vec<PathBuf> {
    conflicts
        .iter()
        .filter_map(|conflict| match conflict {
            Conflict::Ofme(path) | Conflict::SteamFix(path) => Some(path.clone()),
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
    if OFME_STRONG.iter().any(|n| *n == lower)
        || OFME_SUPPORT.iter().any(|n| *n == lower)
        || has_ofme_stem(&lower)
    {
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

fn display_files(files: &[PathBuf]) -> String {
    files
        .iter()
        .filter_map(|p| p.file_name()?.to_str())
        .collect::<Vec<_>>()
        .join(", ")
}
