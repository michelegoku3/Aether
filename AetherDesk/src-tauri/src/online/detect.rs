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
use crate::online::steamstub::detect_steamstub;
use crate::online::types::{BackendReport, Conflict, DetectionReport, Engine, GameArch, PhotonFlavor};
use std::fs;
use std::path::{Path, PathBuf};

/// Nome file dell'early overlay proxy in base all'engine (x64 only).
pub struct OverlayTarget {
    pub file_name: &'static str,
    pub path: PathBuf,
}

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

/// Proxy DLL generici dietro cui un fix può nascondersi (solo se piccoli).
/// `version.dll` e `XINPUT1_3.dll` NON sono qui: dal 1.19.5 sono i nomi
/// dell'overlay proxy UCO2 e li gestisce il deployer, non la quarantena.
const PROXY_DLLS: &[&str] = &["dxgi.dll", "dsound.dll", "winhttp.dll"];

/// Nomi riservati all'early overlay proxy (non vanno trattati come conflitto).
const OVERLAY_PROXY_NAMES: &[&str] = &["version.dll", "xinput1_3.dll"];

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

        // Generic: la DLL Steamworks può essere x64 O x86 (SpeedRunners e
        // molti altri titoli 32-bit hanno solo steam_api.dll, senza _Data
        // né Shipping exe). steam_api_dir_for copre entrambe.
        if let Some(steam_api_dir) = Self::steam_api_dir_for(root, None) {
            return Ok(Self::report_for_generic(root, &steam_api_dir));
        }

        Err(DetectionError::UnrecognizedGame(format!(
            "Could not identify the game in '{}': looked for a '<Game>_Data' folder \
             (Unity), a '*-Win64-Shipping.exe' (Unreal) and any steam_api(64).dll, \
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

        // Convenzione Unity: `<Game>_Data` sta accanto a `<Game>.exe`.
        let game_name = unity_game_name(data_dir);
        let game_exe = Self::pick_game_exe(&ini_dir, &game_name);
        let steamstub_detected = game_exe
            .as_deref()
            .map(detect_steamstub)
            .unwrap_or(false);

        DetectionReport {
            game_root: root.to_path_buf(),
            engine: Engine::Unity,
            arch,
            game_exe,
            unity_data_dir: Some(data_dir.to_path_buf()),
            steam_api_dir,
            ini_dir,
            backends,
            conflicts,
            steamless_applied,
            steamstub_detected,
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
        let steamstub_detected = detect_steamstub(shipping_exe);

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
            steamstub_detected,
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

        let game_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let game_exe = Self::pick_game_exe(steam_api_dir, &game_name);
        let steamstub_detected = game_exe
            .as_deref()
            .map(detect_steamstub)
            .unwrap_or(false);

        DetectionReport {
            game_root: root.to_path_buf(),
            engine: Engine::Generic,
            arch,
            game_exe,
            unity_data_dir: None,
            steam_api_dir: Some(steam_api_dir.to_path_buf()),
            ini_dir: steam_api_dir.to_path_buf(),
            backends,
            conflicts,
            steamless_applied,
            steamstub_detected,
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
    /// OFME/SteamFix: albero intero (i pack Unreal stanno accanto al
    /// Shipping exe, non accanto a steam_api). Proxy generici: solo
    /// accanto alla DLL Steamworks, come prima.
    fn detect_conflicts(root: &Path, steam_api_dir: Option<&Path>) -> Vec<Conflict> {
        let mut conflicts = Vec::new();

        let cold_client = root.join(COLD_CLIENT_INI);
        if cold_client.is_file() {
            conflicts.push(Conflict::ColdClientLoader(cold_client));
        }

        let foreign = crate::online::foreign::scan(root);
        for path in &foreign.files {
            conflicts.push(crate::online::foreign::conflict_for_ofme_file(path));
        }

        let Some(dir) = steam_api_dir else {
            return conflicts;
        };

        // winmm accanto a steam_api è un loader concorrente da quarantinare
        // al deploy, anche senza sibling OFME (che da solo non è OFME).
        for name in ["winmm.dll", "winmm.ini", "winmm.txt"] {
            let path = dir.join(name);
            if path.is_file()
                && !conflicts.iter().any(|c| matches!(c, Conflict::NamedFixFile(p) if p == &path))
            {
                conflicts.push(Conflict::NamedFixFile(path));
            }
        }

        let named_fix_proxy = conflicts.iter().any(|conflict| match conflict {
            Conflict::NamedFixFile(path) => path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.eq_ignore_ascii_case("winmm.dll"))
                .unwrap_or(false),
            _ => false,
        });

        // Proxy generici solo se NON c'è già un loader nominativo (winmm):
        // altrimenti si rischia di toccare lo shim overlay di UCO2.
        if !named_fix_proxy {
            for name in PROXY_DLLS {
                let path = dir.join(name);
                if is_overlay_proxy_name(&path) {
                    continue;
                }
                if path.is_file()
                    && fs::metadata(&path)
                        .map(|meta| meta.len() < UCO_PROXY_MAX_BYTES)
                        .unwrap_or(false)
                {
                    conflicts.push(Conflict::ProxyDll(path));
                }
            }
        }

        conflicts
    }

    /// Avvisi fattuali per la UI (mai blocchi: le decisioni sono dell'utente).
    fn warnings_for(root: &Path, arch: GameArch, backends: &BackendReport) -> Vec<String> {
        let mut warnings = Vec::new();

        if root.join(COLD_CLIENT_INI).is_file() {
            warnings.push(
                "Detected a gbe/ColdClientLoader setup (steamclient64.ini): launch the \
                 real game exe, not the loader. UCOnline2 is a passthrough and needs \
                 the real Steam client."
                    .to_string(),
            );
        }

        if arch == GameArch::X86 {
            warnings.push(
                "32-bit game: the x86 build (steam_api.dll) will be installed."
                    .to_string(),
            );
        }

        if !backends.has_any() {
            warnings.push(
                "No secondary backend detected: if multiplayer is pure Steam P2P \
                 (lobbies/P2P) no plugin is needed, test it bare first."
                    .to_string(),
            );
        }

        warnings
    }

    /// Eseguibile principale del gioco in `dir`, scelto con euristiche che
    /// evitano i falsi positivi (UnityCrashHandler64.exe è più grande di
    /// REPO.exe ma non è il gioco):
    ///   1. stem che corrisponde ESATTAMENTE a `expected_name`
    ///      (convenzione Unity: `<Game>_Data` sta accanto a `<Game>.exe`);
    ///   2. stem che contiene `expected_name` o viceversa;
    ///   3. il più grande tra i candidati non-helper;
    ///   4. fallback: il più grande in assoluto (mai None se c'è un .exe).
    fn pick_game_exe(dir: &Path, expected_name: &str) -> Option<PathBuf> {
        let all = list_exes(dir);
        let playable: Vec<PathBuf> = all.iter().filter(|p| !is_non_game_exe(p)).cloned().collect();
        let pool = if playable.is_empty() { &all } else { &playable };

        if let Some(exact) = pool.iter().find(|p| exe_stem_eq(p, expected_name)) {
            return Some(exact.clone());
        }
        if let Some(partial) = pool.iter().find(|p| exe_stem_contains(p, expected_name)) {
            return Some(partial.clone());
        }
        biggest_exe(pool)
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

/// Sottostringhe che identificano eseguibili che non sono MAI il gioco:
/// helper di engine (UnityCrashHandler*, CrashReportClient*), disinstallatori
/// (unins000.exe) e installer. La sottostringa è volutamente generica per
/// coprire tutte le varianti conosciute.
fn is_non_game_exe(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    const NON_GAME_SUBSTR: &[&str] = &[
        "unitycrashhandler",
        "crashreport",
        "unins",
        "uninstall",
        "installer",
        "setup",
    ];
    NON_GAME_SUBSTR.iter().any(|marker| name.contains(marker))
}

/// `.exe` presenti direttamente in `dir`.
fn list_exes(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut exes: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("exe"))
                .unwrap_or(false)
        })
        .collect();
    exes.sort();
    exes
}

/// Il `.exe` più grande tra i candidati (0 candidati => None).
fn biggest_exe(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .max_by_key(|path| fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .cloned()
}

/// Stem dell'exe uguale (case-insensitive) al nome atteso.
fn exe_stem_eq(path: &Path, expected: &str) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|stem| stem.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

/// Stem dell'exe che contiene il nome atteso, o viceversa (case-insensitive).
fn exe_stem_contains(path: &Path, expected: &str) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let stem_l = stem.to_ascii_lowercase();
    let expected_l = expected.to_ascii_lowercase();
    !expected_l.is_empty() && (stem_l.contains(&expected_l) || expected_l.contains(&stem_l))
}

/// Nome del gioco dalla cartella Unity `<Game>_Data` (stem senza `_Data`).
fn unity_game_name(data_dir: &Path) -> String {
    data_dir
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|stem| stem.strip_suffix(UNITY_DATA_SUFFIX).unwrap_or(stem).to_string())
        .unwrap_or_default()
}

fn is_overlay_proxy_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| OVERLAY_PROXY_NAMES.iter().any(|wanted| n.eq_ignore_ascii_case(wanted)))
        .unwrap_or(false)
}

fn looks_like_phasmophobia(detection: &DetectionReport) -> bool {
    let name_hit = |path: &Path| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase().contains("phasmophobia"))
            .unwrap_or(false)
    };
    name_hit(&detection.game_root)
        || detection.game_exe.as_deref().map(name_hit).unwrap_or(false)
        || detection
            .ini_dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_ascii_lowercase().contains("phasmophobia"))
            .unwrap_or(false)
}

/// Destinazione dell'early overlay proxy, o il motivo dello skip.
///
/// Unity → `version.dll` accanto all'exe; Unreal → `XINPUT1_3.dll` accanto
/// al Shipping exe. x64 only. Phasmophobia è in denylist (inventaria i file).
pub fn overlay_target(detection: &DetectionReport) -> Result<OverlayTarget, &'static str> {
    if detection.arch != GameArch::X64 {
        return Err("Overlay proxy is x64-only.");
    }
    if looks_like_phasmophobia(detection) {
        return Err("Phasmophobia rejects an extra version.dll; overlay proxy skipped.");
    }
    match detection.engine {
        Engine::Unity => Ok(OverlayTarget {
            file_name: "version.dll",
            path: detection.ini_dir.join("version.dll"),
        }),
        Engine::Unreal => {
            let dir = detection
                .game_exe
                .as_ref()
                .and_then(|exe| exe.parent())
                .unwrap_or(&detection.ini_dir);
            Ok(OverlayTarget {
                file_name: "XINPUT1_3.dll",
                path: dir.join("XINPUT1_3.dll"),
            })
        }
        Engine::Generic => Err("No early overlay proxy rule for generic games."),
    }
}
