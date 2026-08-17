//! Locazione e validazione del bundle UCOnline2.
//!
//! Il bundle vive in `ExternalTools/UCOnline2/` (vendored accanto a Steamless)
//! con il layout della release ufficiale:
//!
//! ```text
//! UCOnline2/
//! ├── VERSION              ← tag della release (es. "v1.19.3")
//! ├── x86/steam_api.dll
//! ├── x64/steam_api64.dll
//! └── plugins/*.dll
//! ```
//!
//! Modulo PURO (nessuna dipendenza Tauri): la locazione con `AppHandle`
//! (via `external_tools::bundle::ToolBundleLocator`) è delegata al layer
//! dei comandi, che usa `is_valid_dir` come validatore.

use crate::online::types::GameArch;
use std::path::{Path, PathBuf};

const X64_SUBDIR: &str = "x64";
const X86_SUBDIR: &str = "x86";
const PLUGINS_SUBDIR: &str = "plugins";
const VERSION_FILE: &str = "VERSION";

/// Nomi file attesi (case-insensitive sul filesystem Windows).
const STEAM_API64_DLL: &str = "steam_api64.dll";
const STEAM_API_DLL: &str = "steam_api.dll";

/// Handle su un bundle UCOnline2 valido.
#[derive(Debug, Clone)]
pub struct Uco2Bundle {
    dir: PathBuf,
}

impl Uco2Bundle {
    /// Apre un bundle dalla sua directory radice. `Err` se non valido.
    pub fn open(dir: PathBuf) -> Result<Self, String> {
        if !Self::is_valid_dir(&dir) {
            return Err(format!(
                "UCOnline2 bundle not valid at {}: expected x64/steam_api64.dll, \
                 x86/steam_api.dll and a plugins/ folder with at least one .dll.",
                dir.display()
            ));
        }
        Ok(Self { dir })
    }

    /// True quando la directory contiene un bundle UCOnline2 utilizzabile.
    pub fn is_valid_dir(dir: &Path) -> bool {
        dir.join(X64_SUBDIR).join(STEAM_API64_DLL).is_file()
            && dir.join(X86_SUBDIR).join(STEAM_API_DLL).is_file()
            && plugins_dir_has_dlls(&dir.join(PLUGINS_SUBDIR))
    }

    /// Versione del bundle (dal file VERSION, es. "v1.19.3"), se presente.
    pub fn version(&self) -> Option<String> {
        let path = self.dir.join(VERSION_FILE);
        let content = std::fs::read_to_string(path).ok()?;
        let version = content.trim();
        if version.is_empty() {
            None
        } else {
            Some(version.to_string())
        }
    }

    /// Path della DLL Steamworks da installare per l'architettura data.
    pub fn steam_api_dll(&self, arch: GameArch) -> PathBuf {
        let subdir = match arch {
            GameArch::X64 => X64_SUBDIR,
            GameArch::X86 => X86_SUBDIR,
        };
        self.dir.join(subdir).join(arch.steam_api_file_name())
    }

    /// Path di un plugin per nome (case-insensitive), o `None` se assente.
    ///
    /// I nomi dei plugin nella release (es. `EOS_custom.dll`,
    /// `photon_universal.dll`) non hanno un case garantito: il match è
    /// case-insensitive per essere robusto.
    pub fn plugin_dll(&self, name: &str) -> Option<PathBuf> {
        let wanted = name.to_ascii_lowercase();
        let plugins_dir = self.dir.join(PLUGINS_SUBDIR);
        let entries = std::fs::read_dir(&plugins_dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = file_name.to_ascii_lowercase();
            let name_lower = lower
                .strip_suffix(".dll")
                .map(str::to_string)
                .unwrap_or(lower);
            if name_lower == wanted {
                return Some(path);
            }
        }
        None
    }
}

/// True quando `plugins/` esiste e contiene almeno una `.dll`.
fn plugins_dir_has_dlls(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("dll"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
