//! UCOnline2 engine domain types (`online`).
//!
//! No Tauri dependency: these structs travel between the pure engine and the
//! Tauri commands, and are serialized to the UI. Rule: modules in this crate
//! reference `crate::online::types` ONLY here.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Detected game engine (UCOnline2 patch.bat rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Unity,
    Unreal,
    Generic,
}

/// The game's Steamworks binary architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameArch {
    X64,
    X86,
}

impl GameArch {
    /// Steamworks DLL name to install for this architecture.
    pub fn steam_api_file_name(self) -> &'static str {
        match self {
            GameArch::X64 => "steam_api64.dll",
            GameArch::X86 => "steam_api.dll",
        }
    }
}

/// Detected Photon variant (the `photon_universal` plugin is unique but the
/// config INI differs between Realtime and Fusion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PhotonFlavor {
    #[default]
    None,
    Realtime,
    Fusion,
}

/// Secondary multiplayer backends detected in the game.
///
/// Each flag maps to a UCOnline2 plugin to deploy (or offer to the user)
/// during configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendReport {
    pub photon: PhotonFlavor,
    pub photon_voice: bool,
    pub eos: bool,
    pub playfab: bool,
    pub coherence: bool,
}

impl BackendReport {
    /// True when at least one secondary backend was detected.
    pub fn has_any(&self) -> bool {
        self.photon != PhotonFlavor::None
            || self.photon_voice
            || self.eos
            || self.playfab
            || self.coherence
    }
}

/// Competing emulators/proxies to neutralize (reversible `*.uco-disabled`
/// rename) before deploy, otherwise they fight UCOnline2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "path", rename_all = "camelCase")]
pub enum Conflict {
    /// gbe/ColdClientLoader setup (`steamclient64.ini`): injects its own
    /// steamclient and breaks UCOnline2 passthrough.
    ColdClientLoader(PathBuf),
    /// SteamFix emulator (`SteamFix64.dll`).
    SteamFix(PathBuf),
    /// OFME emulator (`OnlineFix64.dll`) — the online-fix.me crack pack.
    /// OFME and AetherOnline are different stacks: this variant represents
    /// OFME files found on disk, never the Aether mode.
    /// Explicit rename: with rename_all=camelCase "OFME" would become "oFME".
    #[serde(rename = "ofme")]
    Ofme(PathBuf),
    /// Named files of a fix (`winmm.dll`, `dlllist.txt`, ...).
    NamedFixFile(PathBuf),
    /// Small generic proxy DLL (`version.dll`, `dxgi.dll`, ... < 300 KiB).
    ProxyDll(PathBuf),
}

/// Outcome of a game installation inspection: every fact needed to decide
/// whether/how to deploy UCOnline2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionReport {
    /// Game installation root (for DLC harvest and operations).
    pub game_root: PathBuf,
    pub engine: Engine,
    pub arch: GameArch,
    /// Main game executable (Shipping exe for Unreal, otherwise the biggest
    /// next to the Steamworks DLL). Used by the UI and for the "game running"
    /// pre-flight at deploy time.
    pub game_exe: Option<PathBuf>,
    /// For Unity: the `<Game>_Data` folder (with `Managed\` or `il2cpp_data\`).
    pub unity_data_dir: Option<PathBuf>,
    /// Where the UCOnline2 DLL will be installed (where the game already keeps
    /// its `steam_api(64).dll`, falling back to engine conventions).
    pub steam_api_dir: Option<PathBuf>,
    /// Where `union-crax.ini` goes: the running exe's directory (for Unreal
    /// NOT the game root — an ini there would be ignored).
    pub ini_dir: PathBuf,
    pub backends: BackendReport,
    pub conflicts: Vec<Conflict>,
    /// True when Steamless already processed a game executable (Steamless
    /// backup or `.unpacked.exe` output present).
    pub steamless_applied: bool,
    /// True when the main exe has a `.bind` section with a SteamStub 1.x–3.x
    /// signature (GetStubbedLol can hook it at runtime).
    #[serde(default)]
    pub steamstub_detected: bool,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Configuration options (UI payload → enable_online)
// ---------------------------------------------------------------------------

/// Photon options: the user's Photon app GUIDs. Empty = unconfigured
/// (the plugin stays deployed but inert until the ini has the GUIDs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotonOptions {
    pub realtime_guid: String,
    pub voice_guid: String,
    pub fusion_guid: String,
}

/// Credentials of an Epic app (dev.epicgames.com) for EOS_custom.
/// Empty = plugin not deployed (like patch.bat: "detected but not configured").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EosOptions {
    pub product_id: String,
    pub sandbox_id: String,
    pub deployment_id: String,
    pub client_id: String,
    pub client_secret: String,
}

/// The user's PlayFab TitleId. Empty = deployed but inert plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayfabOptions {
    pub title_id: String,
    /// Shared community TitleId (`1D861F`), same pattern as coherence SHARED.
    #[serde(default)]
    pub use_shared: bool,
}

/// Coherence runtime key: your own (project with uploaded schema) or
/// the shared community project (SHARED — availability not guaranteed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoherenceOptions {
    pub runtime_key: String,
    pub use_shared: bool,
}

/// Full online activation request (from the UI panel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineEnableRequest {
    /// The game's REAL AppId (pre-filled by the UI with the library id).
    pub og_app_id: u32,
    /// AppId spoofed in front of Steam (Spacewar).
    pub spoof_app_id: u32,
    /// [Settings] VerboseLog: detailed UCO2 logs (default true in the UI).
    pub verbose_log: bool,
    /// [Settings] EmulateTicket: ticket emulation with ogAppId.
    pub emulate_ticket: bool,
    /// [Settings] WarnOverlayDisabled: warning when the overlay is disabled.
    pub warn_overlay_disabled: bool,
    /// [Settings] SDR: Steam Datagram Relay context for games using it.
    pub sdr: bool,
    /// [Settings] LoadOverlay. Default true (UCO2 loads GameOverlayRenderer).
    #[serde(default = "default_true")]
    pub load_overlay: bool,
    /// [Settings] LogOverlay: writes steam_overlay.log next to the exe.
    #[serde(default)]
    pub log_overlay: bool,
    /// [Settings] GetStubbedLol: runtime SteamStub patch.
    #[serde(default)]
    pub get_stubbed_lol: bool,
    /// [Settings] Client: SteamClient() pin for old SDKs (e.g. "017"). Empty = unset.
    #[serde(default)]
    pub client: String,
    /// [DLC] UnlockAll: unlocks every DLC (default true in the UI).
    pub unlock_all_dlc: bool,
    /// Photon plugin deploy (UI toggle, default off like EOS).
    pub deploy_photon: bool,
    pub photon: PhotonOptions,
    pub eos: EosOptions,
    pub playfab: PlayfabOptions,
    pub coherence: CoherenceOptions,
    /// EOS_custom deploy (default: true when EOS detected).
    pub deploy_eos_custom: bool,
    /// Early overlay proxy deploy (version.dll / XINPUT1_3.dll).
    #[serde(default = "default_true")]
    pub deploy_overlay_proxy: bool,
}

fn default_true() -> bool {
    true
}

impl Default for OnlineEnableRequest {
    fn default() -> Self {
        Self {
            og_app_id: 0,
            spoof_app_id: 480,
            verbose_log: true,
            emulate_ticket: false,
            warn_overlay_disabled: false,
            sdr: false,
            load_overlay: true,
            log_overlay: false,
            get_stubbed_lol: false,
            client: String::new(),
            unlock_all_dlc: true,
            deploy_photon: false,
            photon: PhotonOptions::default(),
            eos: EosOptions::default(),
            playfab: PlayfabOptions::default(),
            coherence: CoherenceOptions::default(),
            deploy_eos_custom: false,
            deploy_overlay_proxy: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Persisted record (per-game UCOnline2 state)
// ---------------------------------------------------------------------------

/// Record of a game with online enabled. Persisted in
/// `<AetherData>/state/uc_online2.json` (key: app_id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineRecord {
    pub app_id: u32,
    /// Epoch timestamp (seconds) of the activation.
    pub enabled_at: u64,
    /// UCO2 bundle version used (e.g. "v1.19.3"), when known.
    pub bundle_version: Option<String>,
    pub og_app_id: u32,
    pub spoof_app_id: u32,
    /// Path of the written `union-crax.ini` (for reconciliation/status).
    pub ini_path: PathBuf,
    /// Path of the installed Steamworks DLL (for reconciliation/status).
    pub steam_api_path: PathBuf,
    pub arch: GameArch,
    /// Deployed plugins (file names, e.g. "photon_universal").
    pub backends_deployed: Vec<String>,
    /// Deploy backup folder path (journal + originals).
    pub backup_dir: PathBuf,
    /// Deployed overlay proxy (`version.dll` / `XINPUT1_3.dll`), when present.
    #[serde(default)]
    pub overlay_proxy_path: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Plan, prerequisites and state
// ---------------------------------------------------------------------------

/// Prerequisites computed at plan time (no disk effects).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prerequisites {
    pub bundle_ok: bool,
    pub bundle_version: Option<String>,
    pub steam_api_dir_writable: bool,
    pub errors: Vec<String>,
}

/// Result of `plan_online`: everything the UI needs to decide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlinePlan {
    pub detection: DetectionReport,
    pub prerequisites: Prerequisites,
    pub current: Option<OnlineRecord>,
    /// Notice ⚠ for the UI (detection warnings + operational notes, one group).
    pub notices: Vec<String>,
}

/// Reconciled state of a game vs UCOnline2 (files on disk are the truth,
/// the record is just an index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineStateKind {
    NotConfigured,
    Enabled,
    /// The record exists but the files (ini/dll) are no longer on disk.
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineStatus {
    pub state: OnlineStateKind,
    pub record: Option<OnlineRecord>,
}

/// Outcome of `enable_online` / `disable_online`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineActionResult {
    pub success: bool,
    pub message: String,
    pub record: Option<OnlineRecord>,
    /// Journal-less Disable: the user must confirm before known UCO2 files are
    /// removed. Not an error, so the UI can show a dedicated confirmation.
    #[serde(default)]
    pub confirmation_required: bool,
    /// Post-action notice (e.g. verify game file integrity after a cleanup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}
