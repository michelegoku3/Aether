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
/// This is intentionally provider-agnostic. Hubcap and Ryuu use ZIP packages;
/// LuaTools may return either the same ZIP shape or a bare pinned Lua file.
pub struct ManifestPackageExtractor;

impl ManifestPackageExtractor {
    /// Parses a provider response that may be either a ZIP package or a bare
    /// Lua file. LuaTools intentionally supports both formats while serving
    /// them from the same download endpoint, so the payload bytes — not the
    /// URL or Content-Disposition filename — are the source of truth.
    pub fn from_provider_bytes(app_id: u32, bytes: &[u8]) -> Result<ManifestPackage, String> {
        if Self::has_zip_signature(bytes) {
            return Self::from_zip(app_id, bytes);
        }

        let content = std::str::from_utf8(bytes)
            .map_err(|_| "Provider returned neither a ZIP archive nor UTF-8 Lua text".to_string())?
            .trim_start_matches('\u{feff}')
            .to_string();
        if !Self::contains_setmanifestid(&content) {
            let preview: String = content
                .chars()
                .take(160)
                .map(|character| if character.is_control() { ' ' } else { character })
                .collect();
            return Err(format!(
                "Provider returned neither a manifest ZIP nor a pinned Lua file. Response starts with: {:?}",
                preview.trim()
            ));
        }

        Ok(ManifestPackage {
            lua_content: content,
            manifest_files: Vec::new(),
        })
    }

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

    fn has_zip_signature(bytes: &[u8]) -> bool {
        bytes.starts_with(b"PK\x03\x04")
            || bytes.starts_with(b"PK\x05\x06")
            || bytes.starts_with(b"PK\x07\x08")
    }

    fn contains_setmanifestid(content: &str) -> bool {
        content.to_ascii_lowercase().contains("setmanifestid")
    }
}
