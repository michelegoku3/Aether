use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
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
    pub image_url: String,
}

#[derive(Debug, Clone)]
pub struct SteamLibraryScanner {
    steam_path: PathBuf,
    active_library: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct AppManifest {
    app_id: u32,
    name: String,
    install_dir: String,
    library_path: PathBuf,
    game_path: PathBuf,
}

#[derive(Debug, Clone)]
struct LuaEntry {
    app_id: u32,
    name: String,
}

impl SteamLibraryScanner {
    pub fn new(steam_path: impl Into<PathBuf>, active_library: Option<String>) -> Self {
        let active_library = active_library
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from);

        Self {
            steam_path: steam_path.into(),
            active_library,
        }
    }

    /// Returns every game represented by a Lua file in Steam/config/stplug-in.
    ///
    /// ACF files are used only as installation metadata. This means a game appears in
    /// Library as soon as Aether/LumaCore has a Lua for it, and the Installed badge is
    /// driven independently by Steam's appmanifest_*.acf files.
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
            let installed = manifest.is_some();
            let name = if !lua.name.trim().is_empty() {
                lua.name
            } else if let Some(manifest) = manifest {
                manifest.name.clone()
            } else {
                format!("App {}", lua.app_id)
            };

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
                image_url: format!(
                    "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/library_600x900.jpg",
                    lua.app_id
                ),
            });
        }

        games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        games
    }

    pub fn is_app_installed(&self, app_id: u32) -> bool {
        let libraries = self.discover_libraries();
        self.scan_appmanifests(&libraries).contains_key(&app_id)
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
        if path.extension().and_then(|ext| ext.to_str()).map(|ext| !ext.eq_ignore_ascii_case("lua")).unwrap_or(true) {
            return None;
        }

        let app_id = path.file_stem()?.to_str()?.parse::<u32>().ok()?;
        let content = fs::read_to_string(path).ok()?;
        let name = Self::extract_game_name_from_lua(&content).unwrap_or_else(|| format!("App {}", app_id));

        Some(LuaEntry { app_id, name })
    }

    fn extract_game_name_from_lua(content: &str) -> Option<String> {
        for line in content.lines() {
            let trimmed = line.trim();
            let Some(comment) = trimmed.strip_prefix("--") else {
                continue;
            };

            let candidate = comment.trim();
            if candidate.is_empty() || Self::is_lua_metadata_comment(candidate) {
                continue;
            }

            return Some(candidate.to_string());
        }

        None
    }

    fn is_lua_metadata_comment(comment: &str) -> bool {
        let lower = comment.to_lowercase();
        lower.contains("'s lua and manifest created")
            || lower.starts_with("created:")
            || lower.starts_with("website:")
            || lower.starts_with("total depots:")
            || lower.starts_with("total dlcs:")
            || lower.starts_with("shared depots:")
            || lower.starts_with("blacklisted depots:")
            || lower.starts_with("depot ")
            || lower.starts_with("main application")
            || lower.starts_with("main app depots")
            || lower.starts_with("shared depots")
            || lower.starts_with("dlcs with dedicated depots")
    }

    fn discover_libraries(&self) -> Vec<PathBuf> {
        let mut libraries = Vec::new();
        Self::push_unique_existing_dir(&mut libraries, self.steam_path.clone());

        if let Some(active_library) = &self.active_library {
            Self::push_unique_existing_dir(&mut libraries, active_library.clone());
        }

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
        path.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase()
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
        let app_id = Self::capture_vdf_value(&content, "appid")?.parse::<u32>().ok()?;
        let name = Self::capture_vdf_value(&content, "name").unwrap_or_default();
        let install_dir = Self::capture_vdf_value(&content, "installdir")?;

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
