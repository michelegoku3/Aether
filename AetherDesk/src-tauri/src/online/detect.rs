//! Ispezione di un'installazione di gioco.
//!
//! Port in Rust delle regole di rilevamento di UCOnline2 `patch.bat`
//! (commit 797d550). Ogni funzione è pura (input: path; output: dati) e
//! non dipende da Tauri: leggibile, testabile con fixture su `tempdir`,
//! riusabile da qualunque front-end.
//!
//! Le regole preservano i casi-bug già risolti da UCOnline2:
//!   * `*_Data` da solo non basta (Farming Simulator ha `web_data`) — serve
//!     il marcatore `Managed\` o `il2cpp_data\`;
//!   * la DLL Steamworks può esistere in più punti (Farming Simulator ha
//!     una copia in root e una in `x64\`) — vince quella accanto all'exe
//!     più grande, non il primo match;
//!   * l'ini va scritto accanto all'EXE IN ESECUZIONE, non nella root
//!     (per Unreal la root non viene mai letta).

use crate::external_tools::constants::{
    is_steamless_backup_name, is_steamless_unpacked_name, UCO_PROXY_MAX_BYTES,
};
use crate::external_tools::fs::{contains_bytes, read_for_scan, walk_dirs, walk_files};
use crate::online::types::{BackendReport, Conflict, DetectionReport, Engine, GameArch, PhotonFlavor};
use std::fs;
use std::path::{Path, PathBuf};

/// Motivo per cui l'ispezione del gioco non è riuscita.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionError {
    /// Nessun marcatore noto (Unity / Unreal / steam_api64.dll).
    UnrecognizedGame(String),
}

impl std::fmt::Display for DetectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DetectionError::UnrecognizedGame(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for DetectionError {}

/// Nomi file/stringhe usati come marcatori (specchia patch.bat).
const STEAM_API64_DLL: &str = "steam_api64.dll";
const STEAM_API_DLL: &str = "steam_api.dll";
const UNITY_DATA_SUFFIX: &str = "_Data";
const UNREAL_SHIPPING_SUFFIX: &str = "-win64-shipping.exe";
const UNREAL_SKIP_SUBSTR: &str = "crashreport";
const COLD_CLIENT_INI: &str = "steamclient64.ini";
const COHERENCE_SCHEMA: &str = "combined.schema";
const EOS_DLL_SHIPPING: &str = "EOSSDK-Win64-Shipping.dll";
const EOS_DLL: &str = "EOSSDK.dll";

/// File nominativi di un fix SteamFix/OnlineFix (sempre da neutralizzare).
const NAMED_FIX_FILES: &[&str] = &[
    "winmm.dll",
    "winmm.txt",
    "winmm.ini",
    "SteamFix64.dll",
    "SteamFix.ini",
    "OnlineFix64.dll",
    "OnlineFix.ini",
    "dlllist.txt",
];

/// Proxy DLL generici dietro cui un fix può nascondersi (solo se piccoli).
const PROXY_DLLS: &[&str] = &["version.dll", "dxgi.dll", "dsound.dll", "winhttp.dll"];

/// Stringhe ANSI nei metadati IL2CPP / negli exe Unreal.
const STR_FUSION: &[u8] = b"NetworkRunner";
const STR_LOAD_BALANCING: &[u8] = b"LoadBalancingClient";
const STR_PHOTON_NETWORK: &[u8] = b"PhotonNetwork";
const STR_PHOTON_VOICE: &[u8] = b"PhotonVoice";
const STR_PLAYFAB_SETTINGS: &[u8] = b"PlayFabSettings";
const STR_COHERENCE_BRIDGE: &[u8] = b"CoherenceBridge";
const STR_EOS_SUBSYSTEM: &[u8] = b"OnlineSubsystemEOS";
const STR_PLAYFAB_SUBSYSTEM: &[u8] = b"OnlineSubsystemPlayFab";
const STR_PHOTON_UNITY_NETWORKING: &[u8] = b"PhotonUnityNetworking";

/// Ispeziona l'installazione di gioco in `root` e produce il report
/// completo dei fatti necessari al deploy di UCOnline2.
pub struct GameInspector;

impl GameInspector {
    pub fn inspect(root: &Path) -> Result<DetectionReport, DetectionError> {
        if let Some(unity_data_dir) = Self::find_unity_data_dir(root) {
            return Ok(Self::report_for_unity(root, &unity_data_dir));
        }

        if let Some(shipping_exe) = Self::find_unreal_shipping_exe(root) {
            return Ok(Self::report_for_unreal(root, &shipping_exe));
        }

        if let Some(steam_api_dir) = Self::pick_steam_api64_dir(root) {
            return Ok(Self::report_for_generic(root, &steam_api_dir));
        }

        Err(DetectionError::UnrecognizedGame(format!(
            "Could not identify the game in '{}': looked for a '<Game>_Data' folder \
             (Unity), a '*-Win64-Shipping.exe' (Unreal) and any steam_api64.dll, \
             and found none of them.",
            root.display()
        )))
    }

    // ------------------------------------------------------------------
    // Engine detection
    // ------------------------------------------------------------------

    /// Prima cartella `<name>_Data` che contiene un marcatore Unity vero
    /// (`Managed\` o `il2cpp_data\`). BFS: la più vicina alla root vince.
    fn find_unity_data_dir(root: &Path) -> Option<PathBuf> {
        walk_dirs(root).into_iter().find(|dir| {
            let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.ends_with(UNITY_DATA_SUFFIX)
                && (dir.join("Managed").is_dir() || dir.join("il2cpp_data").is_dir())
        })
    }

    /// Primo eseguibile `*-Win64-Shipping.exe`, escludendo CrashReport*.
    fn find_unreal_shipping_exe(root: &Path) -> Option<PathBuf> {
        walk_files(root).into_iter().find(|file| {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let lower = name.to_ascii_lowercase();
            lower.ends_with(UNREAL_SHIPPING_SUFFIX) && !lower.contains(UNREAL_SKIP_SUBSTR)
        })
    }

    /// Directory di tutte le `steam_api64.dll` presenti sotto `root`.
    fn steam_api64_dirs(root: &Path) -> Vec<PathBuf> {
        dll_dirs(root, STEAM_API64_DLL)
    }

    /// Directory di tutte le `steam_api.dll` (32-bit) presenti sotto `root`.
    fn steam_api32_dirs(root: &Path) -> Vec<PathBuf> {
        dll_dirs(root, STEAM_API_DLL)
    }

    /// La directory con la `steam_api64.dll` "giusta": quando ce ne sono
    /// più copie, vince quella accanto all'exe più grande (launcher e
    /// server dedicati sono una frazione del gioco vero).
    fn pick_steam_api64_dir(root: &Path) -> Option<PathBuf> {
        let dirs = Self::steam_api64_dirs(root);
        if dirs.is_empty() {
            return None;
        }
        dirs.into_iter()
            .max_by_key(|dir| max_exe_bytes_in(dir))
    }

    // ------------------------------------------------------------------
    // Report per engine
    // ------------------------------------------------------------------

    fn report_for_unity(root: &Path, data_dir: &Path) -> DetectionReport {
        let arch = Self::arch_for(root);
        let steam_api_dir = Self::steam_api_dir_for(
            root,
            // Fallback convenzione Unity: <Data>\Plugins\x86_64\
            Some(data_dir.join("Plugins").join("x86_64")),
        );

        let mut backends = Self::detect_backends_unity(data_dir);
        Self::merge_common_backends(&mut backends, root, steam_api_dir.as_deref());

        let ini_dir = data_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());

        // Derivati calcolati PRIMA che steam_api_dir/backends vengano mossi
        // dentro il report (il borrow checker non ammette usi dopo il move).
        let conflicts = Self::detect_conflicts(root, steam_api_dir.as_deref());
        let warnings = Self::warnings_for(root, arch, &backends);
        let steamless_applied = has_steamless_markers(root);

        DetectionReport {
            game_root: root.to_path_buf(),
            engine: Engine::Unity,
            arch,
            game_exe: Self::biggest_exe_in(&ini_dir),
            unity_data_dir: Some(data_dir.to_path_buf()),
            steam_api_dir,
            ini_dir,
            backends,
            conflicts,
            steamless_applied,
            warnings,
        }
    }

    fn report_for_unreal(root: &Path, shipping_exe: &Path) -> DetectionReport {
        let arch = Self::arch_for(root);
        let exe_dir = shipping_exe
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        // Fallback convenzione Unreal: la dir del Shipping exe.
        let steam_api_dir = Self::steam_api_dir_for(root, Some(exe_dir.clone()));

        let mut backends = BackendReport::default();
        // Stringhe ANSI dei moduli online nell'exe (i literal URL sono
        // UTF-16 e non matchano — stessa nota di patch.bat).
        if let Ok(scan) = read_for_scan(shipping_exe) {
            if contains_bytes(&scan, STR_EOS_SUBSYSTEM) {
                backends.eos = true;
            }
            if contains_bytes(&scan, STR_PLAYFAB_SUBSYSTEM) {
                backends.playfab = true;
            }
            if contains_bytes(&scan, STR_PHOTON_UNITY_NETWORKING) {
                backends.photon = PhotonFlavor::Realtime;
            }
        }
        Self::merge_common_backends(&mut backends, root, steam_api_dir.as_deref());

        let conflicts = Self::detect_conflicts(root, steam_api_dir.as_deref());
        let warnings = Self::warnings_for(root, arch, &backends);
        let steamless_applied = has_steamless_markers(root);

        DetectionReport {
            game_root: root.to_path_buf(),
            engine: Engine::Unreal,
            arch,
            game_exe: Some(shipping_exe.to_path_buf()),
            unity_data_dir: None,
            steam_api_dir,
            ini_dir: exe_dir,
            backends,
            conflicts,
            steamless_applied,
            warnings,
        }
    }

    fn report_for_generic(root: &Path, steam_api_dir: &Path) -> DetectionReport {
        let arch = Self::arch_for(root);
        let mut backends = BackendReport::default();
        Self::merge_common_backends(&mut backends, root, Some(steam_api_dir));

        let conflicts = Self::detect_conflicts(root, Some(steam_api_dir));
        let warnings = Self::warnings_for(root, arch, &backends);
        let steamless_applied = has_steamless_markers(root);

        DetectionReport {
            game_root: root.to_path_buf(),
            engine: Engine::Generic,
            arch,
            game_exe: Self::biggest_exe_in(steam_api_dir),
            unity_data_dir: None,
            steam_api_dir: Some(steam_api_dir.to_path_buf()),
            ini_dir: steam_api_dir.to_path_buf(),
            backends,
            conflicts,
            steamless_applied,
            warnings,
        }
    }

    // ------------------------------------------------------------------
    // Architettura e posizione della DLL
    // ------------------------------------------------------------------

    /// X86 solo quando esiste `steam_api.dll` e nessuna `steam_api64.dll`.
    fn arch_for(root: &Path) -> GameArch {
        if Self::steam_api64_dirs(root).is_empty() && !Self::steam_api32_dirs(root).is_empty() {
            GameArch::X86
        } else {
            GameArch::X64
        }
    }

    /// Dove installare la DLL: la posizione "autoritativa" (dove il gioco
    /// tiene già la sua copia), altrimenti il fallback convenzione passato
    /// dal chiamante.
    fn steam_api_dir_for(root: &Path, convention_fallback: Option<PathBuf>) -> Option<PathBuf> {
        if let Some(best) = Self::pick_steam_api64_dir(root) {
            return Some(best);
        }
        // Gioco 32-bit: prima `steam_api.dll` trovata.
        let dirs = Self::steam_api32_dirs(root);
        if let Some(first) = dirs.first() {
            return Some(first.clone());
        }
        convention_fallback
    }

    // ------------------------------------------------------------------
    // Backend detection
    // ------------------------------------------------------------------

    /// Backend dai contenuti Unity (Mono Managed\ o metadata IL2CPP).
    fn detect_backends_unity(data_dir: &Path) -> BackendReport {
        let mut backends = BackendReport::default();
        let managed = data_dir.join("Managed");
        let metadata = data_dir.join("il2cpp_data").join("Metadata").join("global-metadata.dat");

        if managed.is_dir() {
            // ---- Mono: presenza file negli assembly gestiti ----
            if managed.join("Fusion.Realtime.dll").is_file() {
                backends.photon = PhotonFlavor::Fusion;
            } else if managed.join("PhotonUnityNetworking.dll").is_file()
                || managed.join("PhotonRealtime.dll").is_file()
            {
                backends.photon = PhotonFlavor::Realtime;
            }
            backends.photon_voice = managed.join("PhotonVoice.dll").is_file()
                || managed.join("PhotonVoice.PUN.dll").is_file();
            backends.playfab = managed.join("PlayFabAllSDK.dll").is_file()
                || has_glob(&managed, "PlayFab", ".dll");
            backends.coherence = managed.join("Coherence.Toolkit.dll").is_file();
        } else if metadata.is_file() {
            // ---- IL2CPP: stringhe nel global-metadata.dat ----
            let scan = read_for_scan(&metadata).unwrap_or_default();
            if contains_bytes(&scan, STR_FUSION) {
                backends.photon = PhotonFlavor::Fusion;
            } else if contains_bytes(&scan, STR_LOAD_BALANCING)
                || contains_bytes(&scan, STR_PHOTON_NETWORK)
            {
                backends.photon = PhotonFlavor::Realtime;
            }
            backends.photon_voice = contains_bytes(&scan, STR_PHOTON_VOICE);
            backends.playfab = contains_bytes(&scan, STR_PLAYFAB_SETTINGS);
            backends.coherence = contains_bytes(&scan, STR_COHERENCE_BRIDGE);
        }

        backends
    }

    /// Backend comuni a tutti gli engine (file SDK nel gioco).
    fn merge_common_backends(
        backends: &mut BackendReport,
        root: &Path,
        steam_api_dir: Option<&Path>,
    ) {
        // EOS: SDK nativo ovunque nel gioco (file presence > string scan).
        if !backends.eos {
            backends.eos = walk_files(root).into_iter().any(|file| {
                let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
                name == EOS_DLL_SHIPPING || name == EOS_DLL
            });
        }

        // PlayFab nativo accanto alla DLL Steamworks (No Man's Sky e simili).
        if !backends.playfab {
            if let Some(dir) = steam_api_dir {
                backends.playfab = dir.join("PartyWin.dll").is_file()
                    || has_glob(dir, "PlayFab", ".dll");
            }
        }

        // coherence: lo schema è presente in ogni build, è il marcatore
        // migliore; il file può mancare (schema embedded) — in quel caso i
        // marcatori Unity sopra l'hanno già coperto.
        if !backends.coherence {
            backends.coherence = walk_files(root)
                .into_iter()
                .any(|file| file.file_name().and_then(|n| n.to_str()) == Some(COHERENCE_SCHEMA));
        }
    }

    // ------------------------------------------------------------------
    // Conflitti ed euristiche residue
    // ------------------------------------------------------------------

    /// Emulatori concorrenti da neutralizzare prima del deploy.
    fn detect_conflicts(root: &Path, steam_api_dir: Option<&Path>) -> Vec<Conflict> {
        let mut conflicts = Vec::new();

        let cold_client = root.join(COLD_CLIENT_INI);
        if cold_client.is_file() {
            conflicts.push(Conflict::ColdClientLoader(cold_client));
        }

        let Some(dir) = steam_api_dir else {
            return conflicts;
        };

        for &name in NAMED_FIX_FILES {
            let path = dir.join(name);
            if !path.is_file() {
                continue;
            }
            let conflict = match name {
                "SteamFix64.dll" => Conflict::SteamFix(path),
                "OnlineFix64.dll" => Conflict::OnlineFix(path),
                _ => Conflict::NamedFixFile(path),
            };
            conflicts.push(conflict);
        }

        for name in PROXY_DLLS {
            let path = dir.join(name);
            if path.is_file()
                && fs::metadata(&path)
                    .map(|meta| meta.len() < UCO_PROXY_MAX_BYTES)
                    .unwrap_or(false)
            {
                conflicts.push(Conflict::ProxyDll(path));
            }
        }

        conflicts
    }

    /// Avvisi fattuali per la UI (mai blocchi: le decisioni sono dell'utente).
    fn warnings_for(root: &Path, arch: GameArch, backends: &BackendReport) -> Vec<String> {
        let mut warnings = Vec::new();

        if root.join(COLD_CLIENT_INI).is_file() {
            warnings.push(
                "Rilevato setup gbe/ColdClientLoader (steamclient64.ini): lancia il vero \
                 exe del gioco, non il loader — UCOnline2 è un passthrough e ha bisogno \
                 del client Steam reale."
                    .to_string(),
            );
        }

        if arch == GameArch::X86 {
            warnings.push(
                "Gioco a 32 bit: verrà installata la build x86 (steam_api.dll)."
                    .to_string(),
            );
        }

        if !backends.has_any() {
            warnings.push(
                "Nessun backend secondario rilevato: se il multiplayer è Steam P2P puro \
                 (lobby/P2P) non serve alcun plugin — prova prima senza."
                    .to_string(),
            );
        }

        warnings
    }

    /// True quando Steamless ha già lasciato i suoi marcatori nel gioco.
    fn biggest_exe_in(dir: &Path) -> Option<PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        let mut best: Option<(u64, PathBuf)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
            let is_exe = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("exe"))
                .unwrap_or(false);
            if !is_exe {
                continue;
            }
            let size = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            if best.as_ref().map(|(s, _)| size > *s).unwrap_or(true) {
                best = Some((size, path));
            }
        }
        best.map(|(_, path)| path)
    }
}

// ----------------------------------------------------------------------
// Helper di modulo (private)
// ----------------------------------------------------------------------

fn dll_dirs(root: &Path, dll_name: &str) -> Vec<PathBuf> {
    walk_files(root)
        .into_iter()
        .filter(|file| file.file_name().and_then(|n| n.to_str()) == Some(dll_name))
        .filter_map(|file| file.parent().map(Path::to_path_buf))
        .collect()
}

/// Dimensione massima di un `.exe` direttamente dentro `dir` (0 se nessuno).
fn max_exe_bytes_in(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_exe = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("exe"))
                .unwrap_or(false);
            if !is_exe {
                return None;
            }
            fs::metadata(&path).ok().map(|meta| meta.len())
        })
        .max()
        .unwrap_or(0)
}

/// True quando in `dir` esiste un file con nome `prefix*` + `suffix`.
fn has_glob(dir: &Path, prefix: &str, suffix: &str) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        name.starts_with(prefix) && name.ends_with(suffix)
    })
}

/// True quando un backup Steamless o un output non applicato è presente.
fn has_steamless_markers(root: &Path) -> bool {
    walk_files(root).into_iter().any(|file| {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        is_steamless_backup_name(name) || is_steamless_unpacked_name(name)
    })
}
