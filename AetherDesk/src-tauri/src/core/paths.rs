use std::path::PathBuf;
use tauri::Manager;

const LOCAL_DATA_DIR_NAME: &str = "AetherData";

/// Resolver centralizzato per i path di AetherDesk.
///
/// ## Fix v3 — Tutto unificato in un'unica cartella (09/08/2026)
/// Requisito utente: *l'intera cartella di Aether deve avere tutto dentro di lei*.
/// Non deve esserci un pezzo in `AppData\Local` e uno in `AppData\Roaming`.
/// Soluzione: sia l'eseguibile che `AetherData` vivono dentro la stessa
/// cartella di installazione, che con `installMode=currentUser` è
/// `%LOCALAPPDATA%\AetherDesk\` (scrivibile senza UAC, quindi temi/wallpaper
/// sono editabili senza admin). Struttura:
///
///   %LOCALAPPDATA%\AetherDesk\
///     ├─ AetherDesk.exe
///     ├─ ExternalTools\Steamless\...
///     └─ AetherData\
///         ├─ config\themes\*.css
///         ├─ config\wallpapers\*.jpg
///         ├─ config\settings.json
///         └─ backup\...
///
/// Vecchie location (da migrare):
///   - Legacy Program Files: `<exe_parent>\AetherData` quando l'app era in `C:\Program Files`
///   - Fix v2 Roaming: `%APPDATA%\com.aether.desk` (Roaming)
/// Entrambe vengono migrate automaticamente al primo avvio verso la nuova
/// `install_root/AetherData`.
pub struct LocalAppPaths;

#[allow(dead_code)]
impl LocalAppPaths {
    /// Cartella contenente l'eseguibile.
    /// Con `currentUser` → `%LOCALAPPDATA%\AetherDesk\`
    /// Con `Program Files` (vecchie install) → `C:\Program Files\AetherDesk\`
    pub fn install_root() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Path primario e unico: `<install_root>/AetherData`.
    /// È l'unica location usata da ora in poi.
    pub fn data_root() -> PathBuf {
        Self::install_root().join(LOCAL_DATA_DIR_NAME)
    }

    /// Alias per compatibilità con codice che passava `&AppHandle`.
    /// Ora ignora `app` e ritorna sempre `install_root/AetherData` (unificato).
    pub fn data_root_for_app(_app: &tauri::AppHandle) -> PathBuf {
        Self::data_root()
    }

    pub fn data_root_fallback() -> PathBuf {
        Self::data_root()
    }

    /// Legacy Roaming del fix v2: `%APPDATA%\com.aether.desk`
    /// Mantenuto solo per migrazione di ritorno verso l'install unificata.
    pub fn legacy_roaming_data_root() -> PathBuf {
        if let Some(base) = dirs::data_dir() {
            return base.join("com.aether.desk");
        }
        Self::install_root().join(LOCAL_DATA_DIR_NAME)
    }

    /// Legacy diretta accanto all'exe quando l'installer era `both`/`perMachine`.
    /// Ora coincide con `data_root()` quando l'install è ancora in Program Files,
    /// ma la distinguiamo per la migrazione verso LocalAppData.
    pub fn legacy_program_files_data_root() -> PathBuf {
        Self::install_root().join(LOCAL_DATA_DIR_NAME)
    }

    /// Directory dei tool esterni — resta accanto all'exe (read-only bundled).
    pub fn external_tools_dir() -> PathBuf {
        Self::install_root().join("ExternalTools")
    }

    pub fn steamless_dir() -> PathBuf {
        Self::external_tools_dir().join("Steamless")
    }

    pub fn config_dir() -> PathBuf {
        Self::data_root().join("config")
    }

    pub fn config_dir_for_app(_app: &tauri::AppHandle) -> PathBuf {
        Self::config_dir()
    }

    pub fn temp_dir() -> PathBuf {
        Self::data_root().join("temp")
    }

    pub fn temp_dir_for_app(_app: &tauri::AppHandle) -> PathBuf {
        Self::temp_dir()
    }

    pub fn legacy_app_data_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_data_dir().ok()
    }

    pub fn legacy_app_config_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_config_dir().ok()
    }
}
