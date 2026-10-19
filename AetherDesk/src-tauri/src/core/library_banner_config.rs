//! Sync for the DLL's `[library]` section: the cosmetic "From the library of"
//! banner shown on selected .lua games.
//!
//! AetherDesk owns the settings (`settings.json`); AetherCore reads
//! `aethercore.toml` at Steam start. This module bridges the two:
//!
//! ```toml
//! [library]
//! owner_banner = true
//! owner_banner_apps = [4656000, 4108000]
//! owner_banner_steamid = "76561199876393402"
//! ```
//!
//! The App ID grammar lives in the frontend (`views/settings/libraryBanner.ts`),
//! which normalizes the single-line input into the typed list stored here.

use crate::core::settings::AppSettings;
use std::path::Path;

/// Writes the three `[library]` keys for `settings`. Idempotent: unchanged
/// values leave the document byte-identical (see `config_document::set`).
pub fn sync_library_config(path: &Path, settings: &AppSettings) -> Result<(), String> {
    super::config_document::set_value(
        path,
        "library",
        "owner_banner",
        toml_edit::Value::from(settings.library_banner_enabled),
    )?;

    let mut apps = toml_edit::Array::new();
    for app_id in &settings.library_banner_appids {
        apps.push(i64::from(*app_id));
    }
    super::config_document::set_value(
        path,
        "library",
        "owner_banner_apps",
        toml_edit::Value::Array(apps),
    )?;

    super::config_document::set_value(
        path,
        "library",
        "owner_banner_steamid",
        toml_edit::Value::from(settings.library_banner_steamid.as_str()),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("tmp file");
        f.write_all(content.as_bytes()).expect("write tmp");
        f
    }

    #[test]
    fn sync_creates_section_and_round_trips() {
        let f = write_tmp("[log]\nlevel = \"info\"\n");
        let settings = AppSettings {
            library_banner_enabled: true,
            library_banner_appids: vec![4108000, 4656000],
            library_banner_steamid: "76561199876393402".to_string(),
            ..AppSettings::default()
        };
        sync_library_config(f.path(), &settings).unwrap();

        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("[library]"), "{content}");
        assert!(content.contains("owner_banner = true"), "{content}");
        assert!(
            content.contains("owner_banner_apps = [4108000, 4656000]"),
            "{content}"
        );
        assert!(
            content.contains("owner_banner_steamid = \"76561199876393402\""),
            "{content}"
        );

        // Idempotent: a second sync is a no-op.
        sync_library_config(f.path(), &settings).unwrap();
        assert_eq!(content, std::fs::read_to_string(f.path()).unwrap());
    }

    #[test]
    fn sync_disables_with_empty_values() {
        let f = write_tmp("[library]\nowner_banner = true\nowner_banner_apps = [1]\n");
        let settings = AppSettings::default();
        sync_library_config(f.path(), &settings).unwrap();

        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("owner_banner = false"), "{content}");
        assert!(content.contains("owner_banner_apps = []"), "{content}");
        assert!(content.contains("owner_banner_steamid = \"\""), "{content}");
    }
}
