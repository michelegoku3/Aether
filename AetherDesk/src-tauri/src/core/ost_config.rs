//! Thin policy adapter; shared document editor owns all I/O and locking.
pub fn aethercore_toml_paths(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    super::presence_config::aethercore_toml_paths(app)
}
pub fn read_ost_enabled(path: &std::path::Path) -> Option<bool> {
    let doc = super::config_document::read(path)?;
    doc.get("network")?.get("use_ost_source")?.as_bool()
}
pub fn set_ost_enabled_in_toml(path: &std::path::Path, enabled: bool) -> Result<bool, String> {
    super::config_document::set_value(
        path,
        "network",
        "use_ost_source",
        toml_edit::Value::from(enabled),
    )
}
pub fn ensure_defaults(path: &std::path::Path) {
    let _ = super::config_document::ensure_value(
        path,
        "network",
        "use_ost_source",
        toml_edit::Value::from(false),
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
        let f = write_tmp("[network]\npattern_mirror = \"\"\n");
        assert_eq!(read_ost_enabled(f.path()), None);
    }

    #[test]
    fn set_creates_network_section_when_missing() {
        let f = write_tmp("[log]\nlevel = \"trace\"\n");
        assert!(set_ost_enabled_in_toml(f.path(), true).unwrap());
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("[network]"));
        assert!(content.contains("use_ost_source = true"));
        assert_eq!(read_ost_enabled(f.path()), Some(true));
    }

    #[test]
    fn set_is_idempotent_and_keeps_comments() {
        let f = write_tmp("[network]\n# keep me\nuse_ost_source = false  # user choice\n");
        assert!(!set_ost_enabled_in_toml(f.path(), false).unwrap());
        assert!(set_ost_enabled_in_toml(f.path(), true).unwrap());
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("# keep me"));
        assert!(content.contains("# user choice"));
        assert_eq!(read_ost_enabled(f.path()), Some(true));
    }

    #[test]
    fn ensure_defaults_does_not_override_true() {
        let f = write_tmp("[network]\nuse_ost_source = true\n");
        ensure_defaults(f.path());
        assert_eq!(read_ost_enabled(f.path()), Some(true));
    }

    #[test]
    fn ensure_defaults_inserts_false_when_missing() {
        let f = write_tmp("[network]\npattern_mirror = \"\"\n");
        ensure_defaults(f.path());
        assert_eq!(read_ost_enabled(f.path()), Some(false));
    }
}
