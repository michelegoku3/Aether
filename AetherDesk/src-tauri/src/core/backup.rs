// Centralized per-game backup layout under AetherData.
//
// Target layout (single source of truth for everything a game may need to
// restore or re-apply later):
//
//   <AetherData>/backup/<app_id>/
//       lua/        <app_id>.lua + any bundled Steam .manifest files
//       original/   original game files replaced by the crack, plus a text file
//                   listing every file the crack adds (crack "inventory")
//       crack/      the crack files themselves (reused if still working)
//
// This module owns creating that structure and writing into it. It is a pure
// filesystem service: it takes no AppHandle and knows nothing about Steam, so
// it stays small, testable and decoupled from the Tauri command layer.
use crate::core::paths::LocalAppPaths;
use crate::external_tools::fs::write_atomic;
use crate::manifest::package::ManifestPackageFile;
use std::fs;
use std::path::{Path, PathBuf};

const BACKUP_ROOT: &str = "backup";
const LUA_SUBDIR: &str = "lua";
const ORIGINAL_SUBDIR: &str = "original";
const CRACK_SUBDIR: &str = "crack";

pub struct GameBackup {
    root: PathBuf,
}

impl GameBackup {
    /// Build a `GameBackup` handle for an app, creating the `backup/<app_id>/`
    /// tree (with `lua`, `original` and `crack` sub-folders) on first use.
    pub fn for_app(app_id: u32) -> Result<Self, String> {
        let root = LocalAppPaths::data_root()
            .join(BACKUP_ROOT)
            .join(app_id.to_string());

        for sub in [LUA_SUBDIR, ORIGINAL_SUBDIR, CRACK_SUBDIR] {
            fs::create_dir_all(root.join(sub)).map_err(|error| {
                format!("Failed to create backup folder {}: {}", root.join(sub).display(), error)
            })?;
        }

        Ok(Self { root })
    }

    pub fn lua_dir(&self) -> PathBuf {
        self.root.join(LUA_SUBDIR)
    }

    pub fn original_dir(&self) -> PathBuf {
        self.root.join(ORIGINAL_SUBDIR)
    }

    pub fn crack_dir(&self) -> PathBuf {
        self.root.join(CRACK_SUBDIR)
    }

    /// Path of the crack inventory file (`original/crack_<app_id>.txt`).
    pub fn crack_inventory_path(&self, app_id: u32) -> PathBuf {
        self.original_dir().join(format!("crack_{}.txt", app_id))
    }

    /// Open an existing backup tree without creating folders.
    /// Returns `None` when `backup/<app_id>` does not exist yet.
    pub fn open_existing(app_id: u32) -> Option<Self> {
        let root = LocalAppPaths::data_root()
            .join(BACKUP_ROOT)
            .join(app_id.to_string());
        if root.is_dir() {
            Some(Self { root })
        } else {
            None
        }
    }

    /// True when `backup/<app_id>/crack/` contains at least one file.
    pub fn has_saved_crack(&self) -> bool {
        dir_has_files(&self.crack_dir())
    }

    /// Recursively list files under `crack/` as paths relative to the crack dir
    /// (same layout as game-relative paths used when the crack was applied).
    pub fn list_saved_crack_files(&self) -> Result<Vec<String>, String> {
        let crack_dir = self.crack_dir();
        if !crack_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        collect_relative_files(&crack_dir, &crack_dir, &mut files)?;
        files.sort();
        Ok(files)
    }

    /// Read the crack inventory (game-relative paths, one per line).
    pub fn read_crack_inventory(&self, app_id: u32) -> Result<Vec<String>, String> {
        let path = self.crack_inventory_path(app_id);
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path).map_err(|error| {
            format!("Failed to read crack inventory {}: {}", path.display(), error)
        })?;
        Ok(content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect())
    }

    /// Clear the crack inventory after the applied crack has been removed from the game.
    pub fn clear_crack_inventory(&self, app_id: u32) -> Result<(), String> {
        let path = self.crack_inventory_path(app_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                format!("Failed to remove crack inventory {}: {}", path.display(), error)
            })?;
        }
        Ok(())
    }

    /// Persist the Lua and any bundled Steam `.manifest` files for a game.
    ///
    /// This is the central "Lua backup" step: it is called every time a Lua is
    /// downloaded/installed, so the game's Lua manifest source is always kept.
    /// Writes are atomic (temp file + rename) to avoid a partially-written file
    /// if the app is interrupted.
    pub fn backup_lua_artifacts(
        &self,
        app_id: u32,
        lua_content: &str,
        manifest_files: &[ManifestPackageFile],
    ) -> Result<(), String> {
        // Un download dalle fonti è la versione PRISTINA: diventa/sostituisce
        // l'originale nel backup (quella precedente finisce in history/).
        self.store_original(app_id, lua_content.as_bytes())?;

        for manifest in manifest_files {
            let manifest_path = self.lua_dir().join(&manifest.file_name);
            write_atomic(&manifest_path, &manifest.bytes)?;
        }

        Ok(())
    }
}

fn dir_has_files(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                return true;
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    false
}

fn collect_relative_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("Failed to read folder {}: {}", dir.display(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Failed to read folder entry: {}", error))?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(root, &path, out)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root).map_err(|_| {
                format!(
                    "Internal error: file {} is outside {}",
                    path.display(),
                    root.display()
                )
            })?;
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

// ============================================================================
// Sincronizzazione all'avvio: stplug-in -> backup/<app_id>/lua
// ============================================================================
// Il backup lua viene scritto normalmente da `backup_lua_artifacts` quando un
// .lua passa dall'app, ma l'utente può anche aggiungere o modificare i .lua a
// mano in `config\stplug-in` (o cambiarli con Change Version). Questa funzione,
// lanciata all'avvio in background, riporta il backup allo stato attuale:
//
//   * .lua senza backup        -> backup creato
//   * backup diverso dal .lua  -> la versione precedente viene archiviata in
//                                 `lua\history\<app_id>-<unix_secs>.lua` e il
//                                 backup aggiornato (nessuna "build" persa:
//                                 è la base per la futura scheda "tutte le
//                                 versioni" in Change Version)
//   * backup identico          -> nessuna scrittura (confronto byte; i .lua
//                                 sono piccoli, leggere è più economico e
//                                 più esatto di hash + firma su disco)
//
// I file rimossi da stplug-in NON cancellano il backup: l'ultima versione
// nota resta disponibile (comportamento voluto per un backup).
// La funzione è pura I/O bloccante: avvolgerla in spawn_blocking.

use sha2::{Digest, Sha256};

pub const LUA_HISTORY_SUBDIR: &str = "history";

/// Esito della scrittura di una versione lua nel backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreLuaAction {
    Created,
    Updated,
    Unchanged,
}

/// Esito aggregato della sincronizzazione (log + futuro uso UI).
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct LuaSyncReport {
    pub scanned: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub skipped: usize,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Timestamp con precisione millisecondi: due archivi nella stessa giornata
/// non si sovrascrivono mai (bug della versione a secondi).
fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

impl GameBackup {
    /// Scrive la versione ORIGINALE del .lua (quella pristina scaricata dalle
    /// fonti, o il primo .lua visto in stplug-in quando non c'è backup).
    /// - identica     -> Unchanged, nessuna scrittura
    /// - diversa      -> l'originale precedente viene archiviata in history/
    ///                   e la nuova diventa l'originale (Updated)
    /// - assente      -> Created
    pub fn store_original(&self, app_id: u32, lua_bytes: &[u8]) -> Result<StoreLuaAction, String> {
        let lua_path = self.lua_dir().join(format!("{app_id}.lua"));
        match fs::read(&lua_path) {
            Ok(current) if current == lua_bytes => Ok(StoreLuaAction::Unchanged),
            Ok(old_bytes) => {
                if let Err(e) = self.archive_lua_history(app_id, &old_bytes) {
                    // L'archivio fallito non deve bloccare l'aggiornamento.
                    crate::desk_log_warn!(
                        "backup",
                        "Could not archive previous original Lua for {}: {}",
                        crate::core::logger::format_appid(app_id),
                        e
                    );
                }
                write_atomic(&lua_path, lua_bytes)?;
                let _ = write_atomic(
                    &self.lua_dir().join(format!("{app_id}.sha256")),
                    sha256_hex(lua_bytes).as_bytes(),
                );
                Ok(StoreLuaAction::Updated)
            }
            Err(_) => {
                write_atomic(&lua_path, lua_bytes)?;
                let _ = write_atomic(
                    &self.lua_dir().join(format!("{app_id}.sha256")),
                    sha256_hex(lua_bytes).as_bytes(),
                );
                Ok(StoreLuaAction::Created)
            }
        }
    }

    /// Archivia una VERSIONE MODIFICATA del .lua in `lua\history\` (cambio
    /// versione, modifiche manuali, ecc.). Deduplicata per contenuto: se la
    /// versione è già l'originale o è già in history, non riscrive nulla.
    /// Ritorna true se ha scritto una nuova entry.
    pub fn store_history_version(&self, app_id: u32, lua_bytes: &[u8]) -> Result<bool, String> {
        if self.has_lua_version(app_id, lua_bytes) {
            return Ok(false);
        }
        self.archive_lua_history(app_id, lua_bytes).map(|_| true)
    }

    /// True se `lua_bytes` coincide con l'originale o con una versione già
    /// presente in history (confronto byte, file piccoli).
    pub fn has_lua_version(&self, app_id: u32, lua_bytes: &[u8]) -> bool {
        let original = self.lua_dir().join(format!("{app_id}.lua"));
        if fs::read(&original).is_ok_and(|current| current == lua_bytes) {
            return true;
        }
        let history_dir = self.lua_dir().join(LUA_HISTORY_SUBDIR);
        let Ok(entries) = fs::read_dir(&history_dir) else {
            return false;
        };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lua"))
            .any(|p| fs::read(&p).is_ok_and(|bytes| bytes == lua_bytes))
    }

    fn archive_lua_history(&self, app_id: u32, bytes: &[u8]) -> Result<std::path::PathBuf, String> {
        let history_dir = self.lua_dir().join(LUA_HISTORY_SUBDIR);
        fs::create_dir_all(&history_dir)
            .map_err(|e| format!("mkdir {}: {}", history_dir.display(), e))?;
        let path = history_dir.join(format!("{}-{}.lua", app_id, unix_millis()));
        fs::write(&path, bytes).map_err(|e| format!("write {}: {}", path.display(), e))?;
        crate::desk_log_info!(
            "backup",
            "Archived Lua version for {} -> {}",
            crate::core::logger::format_appid(app_id),
            path.display()
        );
        Ok(path)
    }
}

/// Una voce dello storico delle versioni lua di un app.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LuaHistoryEntry {
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Sincronizza i backup lua leggendo `steam_path\config\stplug-in`.
/// Bloccante: chiamare dentro `spawn_blocking` (vedi main.rs).
pub fn sync_lua_backups_from_stplug_in(steam_path: &Path) -> LuaSyncReport {
    let mut report = LuaSyncReport::default();
    let plugin_dir = steam_path.join("config").join("stplug-in");

    let entries = match fs::read_dir(&plugin_dir) {
        Ok(entries) => entries,
        Err(error) => {
            crate::desk_log_warn!(
                "backup",
                "Lua backup sync skipped: cannot read {}: {}",
                plugin_dir.display(),
                error
            );
            return report;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;   // salta .lua.bak e altro
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(app_id) = stem.parse::<u32>() else {
            report.skipped += 1;   // nome non numerico: non è un app id
            continue;
        };
        report.scanned += 1;

        let src = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                report.skipped += 1;
                continue;
            }
        };

        // Modello: l'ORIGINALE è la versione pristina (primo .lua visto, o
        // l'ultimo scaricato dalle fonti); ogni contenuto diverso trovato in
        // stplug-in è una versione modificata -> history/ (deduplicata per
        // contenuto, così i riavvii non creano copie).
        // Prestazioni: open_existing non crea cartelle; for_app (che crea
        // l'albero) viene usato solo quando c'è davvero qualcosa da scrivere.
        // Per i 58 giochi già allineati il sync fa SOLO letture.
        let original_exists = GameBackup::open_existing(app_id)
            .map(|b| b.lua_dir().join(format!("{app_id}.lua")).exists())
            .unwrap_or(false);
        if !original_exists {
            match GameBackup::for_app(app_id)
                .and_then(|backup| backup.store_original(app_id, &src))
            {
                Ok(_) => report.created += 1,
                Err(_) => report.skipped += 1,
            }
            continue;
        }
        // Fast path: se il contenuto coincide con l'originale non c'è niente
        // da fare (casi più comuni all'avvio) — un solo confronto, zero write.
        let matches_original = GameBackup::open_existing(app_id)
            .and_then(|b| fs::read(b.lua_dir().join(format!("{app_id}.lua"))).ok())
            .is_some_and(|original| original == src);
        if matches_original {
            report.unchanged += 1;
            continue;
        }
        match GameBackup::open_existing(app_id)
            .ok_or_else(|| "backup tree vanished".to_string())
            .and_then(|backup| backup.store_history_version(app_id, &src))
        {
            Ok(true) => report.updated += 1,       // nuova versione -> history
            Ok(false) => report.unchanged += 1,    // già nota (history)
            Err(_) => report.skipped += 1,
        }
    }

    report
}

/// Elenca lo storico delle versioni lua archiviate per un app (per la futura
/// scheda "tutte le build" in Change Version). Include anche il backup corrente
/// come prima voce quando esiste.
pub fn list_lua_history(app_id: u32) -> Result<Vec<LuaHistoryEntry>, String> {
    let backup = GameBackup::for_app(app_id)?;
    let mut out = Vec::new();

    let current = backup.lua_dir().join(format!("{app_id}.lua"));
    if let Ok(bytes) = fs::read(&current) {
        out.push(LuaHistoryEntry {
            file_name: "original".to_string(),
            path: current.display().to_string(),
            size_bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
    }

    let history_dir = backup.lua_dir().join(LUA_HISTORY_SUBDIR);
    if history_dir.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&history_dir)
            .map_err(|e| format!("Failed to read {}: {}", history_dir.display(), e))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lua"))
            .collect();
        entries.sort();   // nome = <app_id>-<millis>: ordine cronologico
        for path in entries {
            if let Ok(bytes) = fs::read(&path) {
                out.push(LuaHistoryEntry {
                    file_name: path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string(),
                    path: path.display().to_string(),
                    size_bytes: bytes.len() as u64,
                    sha256: sha256_hex(&bytes),
                });
            }
        }
    }
    Ok(out)
}
