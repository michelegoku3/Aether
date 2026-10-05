//! Thin policy adapter; shared document editor owns all I/O and locking.
pub fn aethercore_toml_paths(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    super::presence_config::aethercore_toml_paths(app)
}
pub fn read_manifest_restore_enabled(path: &std::path::Path) -> Option<bool> {
    let doc = super::config_document::read(path)?;
    doc.get("manifest_cache")?
        .get("restore_on_startup")?
        .as_bool()
}
pub fn set_manifest_restore_enabled_in_toml(
    path: &std::path::Path,
    enabled: bool,
) -> Result<bool, String> {
    super::config_document::set_value(
        path,
        "manifest_cache",
        "restore_on_startup",
        toml_edit::Value::from(enabled),
    )
}
pub fn ensure_defaults(path: &std::path::Path) {
    let _ = super::config_document::ensure_value(
        path,
        "manifest_cache",
        "restore_on_startup",
        toml_edit::Value::from(true),
    );
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
    fn read_missing_key_returns_none() {
        let f = write_tmp("[manifest_cache]\n");
        assert_eq!(read_manifest_restore_enabled(f.path()), None);
    }

    #[test]
    fn set_creates_manifest_cache_section_when_missing() {
        let f = write_tmp("[log]\nlevel = \"trace\"\n");
        assert!(set_manifest_restore_enabled_in_toml(f.path(), false).unwrap());
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("[manifest_cache]"));
        assert!(content.contains("restore_on_startup = false"));
        assert_eq!(read_manifest_restore_enabled(f.path()), Some(false));
    }

    #[test]
    fn set_is_idempotent_and_keeps_comments() {
        let f =
            write_tmp("[manifest_cache]\n# keep me\nrestore_on_startup = true  # user choice\n");
        assert!(!set_manifest_restore_enabled_in_toml(f.path(), true).unwrap());
        assert!(set_manifest_restore_enabled_in_toml(f.path(), false).unwrap());
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("# keep me"));
        assert!(content.contains("# user choice"));
        assert_eq!(read_manifest_restore_enabled(f.path()), Some(false));
    }

    #[test]
    fn ensure_defaults_does_not_override_false() {
        let f = write_tmp("[manifest_cache]\nrestore_on_startup = false\n");
        ensure_defaults(f.path());
        assert_eq!(read_manifest_restore_enabled(f.path()), Some(false));
    }

    #[test]
    fn ensure_defaults_inserts_true_when_missing() {
        let f = write_tmp("[log]\nlevel = \"trace\"\n");
        ensure_defaults(f.path());
        assert_eq!(read_manifest_restore_enabled(f.path()), Some(true));
    }
}
