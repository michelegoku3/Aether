use std::io::{Cursor, Read};
use zip::ZipArchive;

#[derive(Debug, Clone)]
pub struct ManifestPackageFile {
    pub file_name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ManifestPackage {
    pub lua_content: String,
    pub manifest_files: Vec<ManifestPackageFile>,
}

/// Generic extractor for provider archives containing a SteamTools/LumaCore Lua
/// plus optional Steam `.manifest` files.
///
/// This is intentionally provider-agnostic. Hubcap uses it today; future sources
/// can reuse the same extraction path as long as they deliver a ZIP-like archive
/// with a `.lua` and optional `.manifest` files.
pub struct ManifestPackageExtractor;

impl ManifestPackageExtractor {
    pub fn from_zip(app_id: u32, bytes: &[u8]) -> Result<ManifestPackage, String> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| format!("Failed to open manifest ZIP: {}", e))?;

        let preferred_lua_name = format!("{}.lua", app_id);
        let mut preferred_lua: Option<String> = None;
        let mut first_pinned_lua: Option<String> = None;
        let mut manifest_files = Vec::new();

        for index in 0..archive.len() {
            let mut file = archive
                .by_index(index)
                .map_err(|e| format!("Failed to read ZIP entry {}: {}", index, e))?;

            let path = file.name().replace('\\', "/");
            let lower_path = path.to_ascii_lowercase();
            let file_name = path.rsplit('/').next().unwrap_or(&path).to_string();

            if lower_path.ends_with(".lua") {
                let mut content = String::new();
                file.read_to_string(&mut content)
                    .map_err(|e| format!("Failed to read Lua file from ZIP ({}): {}", path, e))?;

                if !Self::contains_setmanifestid(&content) {
                    continue;
                }

                if file_name.eq_ignore_ascii_case(&preferred_lua_name) {
                    preferred_lua = Some(content);
                } else if first_pinned_lua.is_none() {
                    first_pinned_lua = Some(content);
                }
            } else if lower_path.ends_with(".manifest") {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|e| format!("Failed to read manifest file from ZIP ({}): {}", path, e))?;

                manifest_files.push(ManifestPackageFile { file_name, bytes });
            }
        }

        let lua_content = preferred_lua
            .or(first_pinned_lua)
            .ok_or_else(|| "Manifest ZIP did not contain a Lua file with setManifestid pins".to_string())?;

        Ok(ManifestPackage { lua_content, manifest_files })
    }

    fn contains_setmanifestid(content: &str) -> bool {
        content.to_ascii_lowercase().contains("setmanifestid")
    }
}
