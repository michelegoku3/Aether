//! Tipi di dominio dell'engine UCOnline2 (`online`).
//!
//! Nessuna dipendenza da Tauri: queste strutture viaggiano tra l'engine
//! puro e i comandi Tauri, e vengono serializzate verso la UI. Regola:
//! i moduli di questo crate referenziano `crate::online::types` SOLO qui.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Motore di gioco rilevato (regole di UCOnline2 patch.bat).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Unity,
    Unreal,
    Generic,
}

/// Architettura del binario Steamworks del gioco.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameArch {
    X64,
    X86,
}

impl GameArch {
    /// Nome della DLL Steamworks da installare per questa architettura.
    pub fn steam_api_file_name(self) -> &'static str {
        match self {
            GameArch::X64 => "steam_api64.dll",
            GameArch::X86 => "steam_api.dll",
        }
    }
}

/// Variante Photon rilevata (il plugin `photon_universal` è unico ma il
/// config INI cambia tra Realtime e Fusion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PhotonFlavor {
    #[default]
    None,
    Realtime,
    Fusion,
}

/// Backend di multiplayer secondari rilevati nel gioco.
///
/// Ogni flag corrisponde a un plugin UCOnline2 da deployare (o da offrire
/// all'utente) in fase di configurazione.
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
    /// True quando almeno un backend secondario è stato rilevato.
    pub fn has_any(&self) -> bool {
        self.photon != PhotonFlavor::None
            || self.photon_voice
            || self.eos
            || self.playfab
            || self.coherence
    }

    /// Nomi dei plugin UCOnline2 che i backend rilevati richiedono.
    pub fn required_plugins(&self) -> Vec<&'static str> {
        let mut plugins = Vec::new();
        if self.photon != PhotonFlavor::None {
            plugins.push("photon_universal");
        }
        if self.eos {
            plugins.push("EOS_custom");
        }
        if self.playfab {
            plugins.push("playfab_universal");
        }
        if self.coherence {
            plugins.push("coherence_universal");
        }
        plugins
    }
}

/// Emulatori/proxy concorrenti che vanno neutralizzati (rinomina reversibile
/// `*.uco-disabled`) prima del deploy, altrimenti litigano con UCOnline2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "path", rename_all = "camelCase")]
pub enum Conflict {
    /// Setup gbe/ColdClientLoader (`steamclient64.ini`): inietta la sua
    /// steamclient e rompe il passthrough di UCOnline2.
    ColdClientLoader(PathBuf),
    /// Emulatore SteamFix (`SteamFix64.dll`).
    SteamFix(PathBuf),
    /// Emulatore OnlineFix (`OnlineFix64.dll`).
    OnlineFix(PathBuf),
    /// File nominativi di un fix (`winmm.dll`, `dlllist.txt`, ...).
    NamedFixFile(PathBuf),
    /// Proxy DLL generico piccolo (`version.dll`, `dxgi.dll`, ... < 300 KiB).
    ProxyDll(PathBuf),
}

/// Esito dell'ispezione di un'installazione di gioco: tutti i fatti che
/// servono per decidere se/come deployare UCOnline2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionReport {
    /// Radice dell'installazione del gioco (per harvest DLC e operazioni).
    pub game_root: PathBuf,
    pub engine: Engine,
    pub arch: GameArch,
    /// Eseguibile principale del gioco (Shipping exe per Unreal, altrimenti
    /// il più grande accanto alla DLL Steamworks). Usato per la UI e per il
    /// pre-flight "gioco in esecuzione" in fase di deploy.
    pub game_exe: Option<PathBuf>,
    /// Per Unity: la cartella `<Game>_Data` (con `Managed\` o `il2cpp_data\`).
    pub unity_data_dir: Option<PathBuf>,
    /// Dove andrà installata la DLL UCOnline2 (dove il gioco tiene già la
    /// sua `steam_api(64).dll`, con fallback alle convenzioni engine).
    pub steam_api_dir: Option<PathBuf>,
    /// Dove va scritto `union-crax.ini`: la directory dell'exe in esecuzione
    /// (per Unreal NON è la root del gioco — un ini lì verrebbe ignorato).
    pub ini_dir: PathBuf,
    pub backends: BackendReport,
    pub conflicts: Vec<Conflict>,
    /// True quando Steamless ha già processato un eseguibile del gioco
    /// (backup `.steamstub.bak` o output `.unpacked.exe` presenti).
    pub steamless_applied: bool,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Opzioni di configurazione (payload UI → enable_online)
// ---------------------------------------------------------------------------

/// Opzioni Photon: GUID degli app Photon dell'utente. Vuoto = non configurato
/// (il plugin resta deployato ma inerte finché l'ini non ha i GUID).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotonOptions {
    pub realtime_guid: String,
    pub voice_guid: String,
    pub fusion_guid: String,
}

/// Credenziali di un'app Epic (dev.epicgames.com) per EOS_custom.
/// Vuote = plugin non deployato (come patch.bat: "detected but not configured").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EosOptions {
    pub product_id: String,
    pub sandbox_id: String,
    pub deployment_id: String,
    pub client_id: String,
    pub client_secret: String,
}

/// TitleId PlayFab dell'utente. Vuoto = plugin deployato ma inerte.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayfabOptions {
    pub title_id: String,
}

/// Runtime key coherence: propria (progetto con schema caricato) oppure
/// il progetto community condiviso (SHARED — disponibilità non garantita).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoherenceOptions {
    pub runtime_key: String,
    pub use_shared: bool,
}

/// Richiesta completa di attivazione online (dal pannello UI).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineEnableRequest {
    /// AppId VERO del gioco (precompilato dalla UI con l'id di libreria).
    pub og_app_id: u32,
    /// AppId spoofato davanti a Steam (Spacewar).
    pub spoof_app_id: u32,
    /// GetStubbedLol: patch SteamStub a runtime (fase F7; già scrivibile ora).
    pub steam_stub_patch: bool,
    pub photon: PhotonOptions,
    pub eos: EosOptions,
    pub playfab: PlayfabOptions,
    pub coherence: CoherenceOptions,
    /// Deploy di EOS_custom (default: true se EOS rilevato).
    pub deploy_eos_custom: bool,
}

// ---------------------------------------------------------------------------
// Record persistito (stato UCOnline2 per gioco)
// ---------------------------------------------------------------------------

/// Record di un gioco con online attivo. Persistito in
/// `<AetherData>/state/uc_online2.json` (chiave: app_id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineRecord {
    pub app_id: u32,
    /// Timestamp epoch (secondi) dell'attivazione.
    pub enabled_at: u64,
    /// Versione del bundle UCO2 usata (es. "v1.19.3"), se nota.
    pub bundle_version: Option<String>,
    pub og_app_id: u32,
    pub spoof_app_id: u32,
    pub steam_stub_patch: bool,
    /// Path di `union-crax.ini` scritto (per riconciliazione/status).
    pub ini_path: PathBuf,
    /// Path della DLL Steamworks installata (per riconciliazione/status).
    pub steam_api_path: PathBuf,
    pub arch: GameArch,
    /// Plugin deployati (nomi file, es. "photon_universal").
    pub backends_deployed: Vec<String>,
    /// Path della cartella di backup del deploy (journal + originali).
    pub backup_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// Piano, prerequisiti e stato
// ---------------------------------------------------------------------------

/// Prerequisiti calcolati in fase di piano (nessun effetto sul disco).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prerequisites {
    pub bundle_ok: bool,
    pub bundle_version: Option<String>,
    pub steam_api_dir_writable: bool,
    pub errors: Vec<String>,
}

/// Risultato di `plan_online`: tutto ciò che serve alla UI per decidere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlinePlan {
    pub detection: DetectionReport,
    pub prerequisites: Prerequisites,
    pub current: Option<OnlineRecord>,
    pub suggestions: Vec<String>,
}

/// Stato riconciliato di un gioco rispetto a UCOnline2 (file sul disco =
/// verità, il record è solo un indice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineStateKind {
    NotConfigured,
    Enabled,
    /// Il record esiste ma i file (ini/dll) non sono più sul disco.
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineStatus {
    pub state: OnlineStateKind,
    pub record: Option<OnlineRecord>,
}

/// Esito di `enable_online` / `disable_online`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineActionResult {
    pub success: bool,
    pub message: String,
    pub record: Option<OnlineRecord>,
}
