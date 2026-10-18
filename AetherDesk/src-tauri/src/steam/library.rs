use crate::manifest::pins::LuaManifestRowIssue;
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSteamGame {
    pub id: u32,
    pub name: String,
    pub app_id: String,
    pub install_dir: String,
    pub library_path: String,
    pub game_path: String,
    pub installed: bool,
    /// Steam is still writing this app's content (`StateFlags` says so).
    /// Mutually exclusive with `installed`: a game is either complete,
    /// in flight, or absent from Steam.
    pub installing: bool,
    pub image_url: String,
    pub hero_image_url: String,
    /// Malformed `setManifestid` lines found in this game's Lua while scanning.
    ///
    /// A single bad line makes AetherDLL reject the **whole file**, so the game
    /// silently loses every depot override. The scanner reads each Lua anyway,
    /// so the diagnosis travels with the card (red marker + editable popup)
    /// instead of requiring a second per-game command.
    pub lua_issues: Vec<LuaManifestRowIssue>,
}

impl InstalledSteamGame {
    /// True when Steam holds an `appmanifest` for this App ID: the content is
    /// either complete (`installed`) or still being written (`installing`).
    ///
    /// This is "Steam knows this app and its real install folder" — the same
    /// meaning as [`SteamLibraryScanner::is_app_installed`] — and it is what
    /// callers that only need the folder need, never `installed` alone.
    pub fn has_appmanifest(&self) -> bool {
        self.installed || self.installing
    }
}

#[derive(Debug, Clone)]
pub struct SteamLibraryScanner {
    steam_path: PathBuf,
}

/// Install-state projection of one Steam `appmanifest_*.acf`, without the
/// Library scan's Lua reads.
///
/// It carries exactly what `scan_installed_games` derives from an ACF, so the
/// filesystem watcher can invalidate the Library when — and only when — the
/// install state a user can see actually changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamInstallEntry {
    pub install_dir: String,
    pub library_path: String,
    pub installed: bool,
}

#[derive(Debug, Clone)]
struct AppManifest {
    app_id: u32,
    name: String,
    install_dir: String,
    library_path: PathBuf,
    game_path: PathBuf,
    /// Steam's `AppState.StateFlags`, when the key is present and numeric.
    state_flags: Option<u64>,
}

/// Steam's own AppState flags that decide whether the content is complete.
/// `Fully Installed` (0x4) is cleared by Steam when it starts writing an
/// install/update; `Update Required` (0x2) marks that same unfinished state.
/// Every other bit (sizes, update scheduling) is irrelevant here.
const STATE_FLAG_UPDATE_REQUIRED: u64 = 0x2;
const STATE_FLAG_FULLY_INSTALLED: u64 = 0x4;

/// True while Steam is still writing this app's content.
///
/// Only the combination "Fully Installed clear + Update Required set" counts:
/// a missing or unexpected value (absent key, `0`) keeps the game as installed.
/// That is the pre-existing behaviour and it can never hide a playable game —
/// while the reversed guess would mark every Aether-pinned version as missing.
fn steam_is_installing(state_flags: Option<u64>) -> bool {
    matches!(
        state_flags,
        Some(flags)
            if (flags & STATE_FLAG_FULLY_INSTALLED) == 0
                && (flags & STATE_FLAG_UPDATE_REQUIRED) != 0
    )
}

#[derive(Debug, Clone)]
struct LuaEntry {
    app_id: u32,
    name: Option<String>,
    /// Malformed pins of this Lua (empty for a healthy file).
    lua_issues: Vec<LuaManifestRowIssue>,
}

impl SteamLibraryScanner {
    pub fn new(steam_path: impl Into<PathBuf>) -> Self {
        // Normalize at the boundary: every consumer stores/uses the same
        // canonical root, so a quoted/padded-but-valid path works everywhere.
        // Callers are still expected to strict-validate at command entry for
        // actionable errors; the scanner itself stays infallible by design.
        let normalized =
            crate::steam::resolve::normalize_steam_path(&steam_path.into().to_string_lossy());
        Self {
            steam_path: PathBuf::from(normalized),
        }
    }

    /// Returns every game represented by a Lua file in Steam/config/stplug-in.
    ///
    /// ACF files are used only as installation metadata. This means a game appears in
    /// Library as soon as Aether/LumaCore has a Lua for it, and the install state is
    /// driven independently by Steam's `appmanifest_*.acf` files: `installed` when
    /// Steam reports the content complete, `installing` while it is still writing it.
    /// The game folder is never inspected: Steam leaves it behind (empty, or holding
    /// leftover crack files) after an uninstall.
    pub fn scan_installed_games(&self) -> Vec<InstalledSteamGame> {
        let libraries = self.discover_libraries();
        let installed_manifests = self.scan_appmanifests(&libraries);
        let lua_entries = self.scan_lua_entries();
        let mut seen = HashSet::new();
        let mut games = Vec::new();

        for lua in lua_entries {
            if !seen.insert(lua.app_id) {
                continue;
            }

            let manifest = installed_manifests.get(&lua.app_id);
            // Steam's StateFlags is the only reliable "content complete" signal:
            // the install folder survives an uninstall and a fresh install
            // populates it only at the end.
            let installing = manifest
                .map(|manifest| steam_is_installing(manifest.state_flags))
                .unwrap_or(false);
            let installed = manifest.is_some() && !installing;
            let name = manifest
                .and_then(|manifest| Self::safe_display_name(&manifest.name))
                .or(lua.name)
                .unwrap_or_else(|| Self::fallback_app_name(lua.app_id));

            games.push(InstalledSteamGame {
                id: lua.app_id,
                name,
                app_id: lua.app_id.to_string(),
                install_dir: manifest.map(|m| m.install_dir.clone()).unwrap_or_default(),
                library_path: manifest
                    .map(|m| m.library_path.display().to_string())
                    .unwrap_or_default(),
                game_path: manifest
                    .map(|m| m.game_path.display().to_string())
                    .unwrap_or_default(),
                installed,
                installing,
                // Never guess a CDN filename. Modern Steam assets are hashed;
                // unhashed library_600x900 / header.jpg either 404 or the UI
                // treats a landscape fallback as a "hero" in the capsule slot.
                // Real URLs come from IStoreBrowseService/GetItems.
                image_url: String::new(),
                hero_image_url: String::new(),
                lua_issues: lua.lua_issues,
            });
        }

        games.sort_by_key(|a| a.name.to_lowercase());
        games
    }

    pub fn is_app_installed(&self, app_id: u32) -> bool {
        let libraries = self.discover_libraries();
        self.scan_appmanifests(&libraries).contains_key(&app_id)
    }

    /// App ID -> install projection for every ACF the scanner accepts.
    ///
    /// Reads only appmanifests (one small file each): the caller is the
    /// filesystem watcher, which compares successive projections on every
    /// event and must not pay for Lua parsing. Download progress never changes
    /// the result — the fields kept here are exactly the ones the Library
    /// exposes, and `installed` is Steam's own "content complete" flag.
    pub fn install_projection(&self) -> BTreeMap<u32, SteamInstallEntry> {
        let libraries = self.discover_libraries();
        self.scan_appmanifests(&libraries)
            .into_iter()
            .map(|(app_id, manifest)| {
                (
                    app_id,
                    SteamInstallEntry {
                        install_dir: manifest.install_dir,
                        library_path: manifest.library_path.display().to_string(),
                        installed: !steam_is_installing(manifest.state_flags),
                    },
                )
            })
            .collect()
    }

    /// Returns every Steam library folder discovered from the configured Steam
    /// path and `libraryfolders.vdf`. Used by the antivirus exclusion command to
    /// ensure crack files written into any library are not quarantined.
    pub fn discover_library_paths(&self) -> Vec<PathBuf> {
        self.discover_libraries()
    }

    fn scan_appmanifests(&self, libraries: &[PathBuf]) -> HashMap<u32, AppManifest> {
        let mut manifests = HashMap::new();

        for library in libraries {
            let steamapps = library.join("steamapps");
            let Ok(entries) = fs::read_dir(&steamapps) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if !Self::is_appmanifest(&path) {
                    continue;
                }

                if let Some(manifest) = Self::parse_appmanifest(&path, library) {
                    manifests.entry(manifest.app_id).or_insert(manifest);
                }
            }
        }

        manifests
    }

    fn scan_lua_entries(&self) -> Vec<LuaEntry> {
        let plugin_dir = self.steam_path.join("config").join("stplug-in");
        let Ok(entries) = fs::read_dir(plugin_dir) else {
            return Vec::new();
        };

        entries
            .flatten()
            .filter_map(|entry| Self::parse_lua_entry(&entry.path()))
            .collect()
    }

    fn parse_lua_entry(path: &Path) -> Option<LuaEntry> {
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| !ext.eq_ignore_ascii_case("lua"))
            .unwrap_or(true)
        {
            return None;
        }

        let app_id = path.file_stem()?.to_str()?.parse::<u32>().ok()?;
        let content = fs::read_to_string(path).ok()?;
        let name = Self::extract_game_name_from_lua(&content);
        // Diagnosis for the whole file: AetherDLL's setmanifestid binding
        // raises on the first malformed call and the script engine rolls the
        // file back, so one bad line disables every depot of this game.
        let lua_issues = crate::manifest::pins::invalid_manifest_rows(&content);

        Some(LuaEntry {
            app_id,
            name,
            lua_issues,
        })
    }

    fn extract_game_name_from_lua(content: &str) -> Option<String> {
        for line in content.lines() {
            let trimmed = line.trim();
            let Some(comment) = trimmed.strip_prefix("--") else {
                continue;
            };

            let candidate = comment.trim();
            if let Some(name) = Self::safe_display_name(candidate) {
                return Some(name);
            }
        }

        None
    }

    fn safe_display_name(candidate: &str) -> Option<String> {
        let name = candidate
            .trim()
            .trim_matches(['-', '=', '*', '#', '/', '\\', ' ']);

        if name.is_empty() || name.len() > 120 || name.chars().count() < 2 {
            return None;
        }

        if name
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch.is_whitespace())
        {
            return None;
        }

        let lower = name.to_lowercase();
        if Self::is_lua_metadata_comment(&lower) || Self::looks_like_url(&lower) {
            return None;
        }

        let symbol_count = name
            .chars()
            .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
            .count();
        if symbol_count > name.chars().count().saturating_div(3).max(6) {
            return None;
        }

        Some(name.to_string())
    }

    fn fallback_app_name(app_id: u32) -> String {
        format!("App ID {}", app_id)
    }

    fn looks_like_url(lower: &str) -> bool {
        lower.contains("://")
            || lower.contains("www.")
            || lower.contains(".com")
            || lower.contains(".net")
            || lower.contains(".org")
    }

    fn is_lua_metadata_comment(lower_comment: &str) -> bool {
        const METADATA_PREFIXES: &[&str] = &[
            "created:",
            "website:",
            "url:",
            "source:",
            "total depots:",
            "total dlcs:",
            "shared depots:",
            "blacklisted depots:",
            "depot ",
            "main application",
            "main app depots",
            "dlcs with dedicated depots",
            "generated",
            "manifest",
            "setmanifestid",
            "addappid",
            "addappid",
        ];

        const METADATA_FRAGMENTS: &[&str] = &[
            " lua ",
            " lua and manifest",
            "'s lua",
            "manifest created",
            "depotcache",
            "steam config",
            "stplug-in",
            "luma",
            "sff",
            "creamapi",
            "decryption key",
            "unlock",
            "depot id",
            "manifest id",
            "branch:",
            "buildid",
            "password",
            "token",
        ];

        lower_comment.starts_with("--")
            || METADATA_PREFIXES
                .iter()
                .any(|prefix| lower_comment.starts_with(prefix))
            || METADATA_FRAGMENTS
                .iter()
                .any(|fragment| lower_comment.contains(fragment))
    }

    fn discover_libraries(&self) -> Vec<PathBuf> {
        let mut libraries = Vec::new();
        Self::push_unique_existing_dir(&mut libraries, self.steam_path.clone());

        // Every extra library is resolved transitively from each discovered
        // `steamapps/libraryfolders.vdf` — no separate "active library"
        // configuration is needed (nor kept).
        let mut index = 0;
        while index < libraries.len() {
            let library = libraries[index].clone();
            for extra in Self::parse_libraryfolders(&library) {
                Self::push_unique_existing_dir(&mut libraries, extra);
            }
            index += 1;
        }

        libraries
    }

    fn push_unique_existing_dir(libraries: &mut Vec<PathBuf>, path: PathBuf) {
        if !path.is_dir() {
            return;
        }

        let normalized = Self::normalize_path(&path);
        let already_present = libraries
            .iter()
            .any(|existing| Self::normalize_path(existing) == normalized);

        if !already_present {
            libraries.push(path);
        }
    }

    fn normalize_path(path: &Path) -> String {
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    }

    fn parse_libraryfolders(steam_root: &Path) -> Vec<PathBuf> {
        let path = steam_root.join("steamapps").join("libraryfolders.vdf");
        let Ok(content) = fs::read_to_string(path) else {
            return Vec::new();
        };

        let Ok(re) = Regex::new(r#"(?i)"path"\s+"([^"]+)""#) else {
            return Vec::new();
        };

        re.captures_iter(&content)
            .filter_map(|captures| captures.get(1))
            .map(|value| PathBuf::from(value.as_str().replace("\\\\", "\\")))
            .collect()
    }

    fn is_appmanifest(path: &Path) -> bool {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };

        file_name.starts_with("appmanifest_") && file_name.ends_with(".acf")
    }

    fn parse_appmanifest(path: &Path, library_path: &Path) -> Option<AppManifest> {
        let content = fs::read_to_string(path).ok()?;
        let app_id = Self::capture_vdf_value(&content, "appid")?
            .parse::<u32>()
            .ok()?;
        let name = Self::capture_vdf_value(&content, "name").unwrap_or_default();
        let install_dir = Self::capture_vdf_value(&content, "installdir")?;
        // Optional by design: an ACF without a parseable StateFlags keeps the
        // game as installed instead of inventing an "installing" state.
        let state_flags = Self::capture_vdf_value(&content, "StateFlags")
            .and_then(|raw| raw.trim().parse::<u64>().ok());

        if install_dir.trim().is_empty() {
            return None;
        }

        let game_path = library_path
            .join("steamapps")
            .join("common")
            .join(&install_dir);

        Some(AppManifest {
            app_id,
            name,
            install_dir,
            library_path: library_path.to_path_buf(),
            game_path,
            state_flags,
        })
    }

    fn capture_vdf_value(content: &str, key: &str) -> Option<String> {
        let escaped_key = regex::escape(key);
        let pattern = format!(r#"(?i)"{}"\s+"([^"]*)""#, escaped_key);
        let re = Regex::new(&pattern).ok()?;
        re.captures(content)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string())
    }
}
