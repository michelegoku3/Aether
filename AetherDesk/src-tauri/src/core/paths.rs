use std::path::PathBuf;
use tauri::Manager;

const LOCAL_DATA_DIR_NAME: &str = "AetherData";

/// Resolver centralizzato per i path di AetherDesk.
///
/// ## Distribuzione portabile (ZIP)
/// AetherDesk è una cartella portabile auto-contenuta: eseguibile, tool e dati
/// vivono tutti insieme. L'intera cartella di Aether ha tutto dentro di lei —
/// nessun pezzo in `AppData\Local` o `AppData\Roaming`. Soluzione: sia
/// l'eseguibile che `AetherData` vivono dentro la stessa cartella di
/// installazione (`install_root/`). Struttura:
///
///   <dove-l'utente-la-sballa>/AetherDesk\
///     ├─ AetherDesk.exe
///     ├─ ExternalTools\Steamless\...
///     └─ AetherData\
///         ├─ config\themes\*.css
///         ├─ config\wallpapers\*.jpg
///         ├─ config\settings.json
///         └─ backup\...
///
/// Vecchie location (da migrare solo per chi arriva da una vecchia install):
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

    /// Legacy Roaming del fix v2: `%APPDATA%\com.aether.desk`
    /// Mantenuto solo per migrazione di ritorno verso l'install unificata.
    pub fn legacy_roaming_data_root() -> PathBuf {
        if let Some(base) = dirs::data_dir() {
            return base.join("com.aether.desk");
        }
        Self::install_root().join(LOCAL_DATA_DIR_NAME)
    }

    /// Directory dei tool esterni — resta accanto all'exe (read-only bundled).
    pub fn external_tools_dir() -> PathBuf {
        Self::install_root().join("ExternalTools")
    }

    pub fn steamless_dir() -> PathBuf {
        Self::external_tools_dir().join("Steamless")
    }

    /// Directory del bundle UCOnline2 (emulatore Steamworks + plugin).
    pub fn uco2_dir() -> PathBuf {
        Self::external_tools_dir().join("UCOnline2")
    }

    pub fn config_dir() -> PathBuf {
        Self::data_root().join("config")
    }

    /// Directory di stato (uc_online2.json e altri stati applicativi).
    pub fn state_dir() -> PathBuf {
        Self::data_root().join("state")
    }

    /// Root dei backup per-gioco (`<AetherData>/backup`).
    pub fn backup_root() -> PathBuf {
        Self::data_root().join("backup")
    }

    pub fn temp_dir() -> PathBuf {
        Self::data_root().join("temp")
    }

    pub fn legacy_app_data_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_data_dir().ok()
    }

    pub fn legacy_app_config_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_config_dir().ok()
    }
}
