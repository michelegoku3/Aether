//! One owner for read/modify/publish of DLL TOML. Lock covers the whole edit,
//! not just rename. No values (custom names, credentials) enter diagnostics.
use std::{fs, path::Path, sync::Mutex};
use toml_edit::{DocumentMut, Item, Value};
static EDIT: Mutex<()> = Mutex::new(());

pub fn edit(
    path: &Path,
    operation: &str,
    change: impl FnOnce(&mut DocumentMut) -> Result<(), String>,
) -> Result<bool, String> {
    let _guard = EDIT
        .lock()
        .map_err(|_| "Configuration editor unavailable")?;
    let _file_lock = super::state_io::lock(&path.with_extension("toml.lock"))?;
    let result = (|| {
        let original = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("Cannot read configuration {}: {e}", path.display())),
        };
        let mut doc = match original.parse::<DocumentMut>() {
            Ok(doc) => doc,
            Err(_) => {
                let mut lines = original.lines().map(str::to_owned).collect::<Vec<_>>();
                if !super::presence_config::dedup_sections(&mut lines) {
                    return Err(format!(
                        "Invalid TOML {}; refusing overwrite",
                        path.display()
                    ));
                }
                let repaired = lines.join("\n") + "\n";
                let doc = repaired.parse::<DocumentMut>().map_err(|_| {
                    format!(
                        "Invalid TOML {}; duplicate-section repair insufficient",
                        path.display()
                    )
                })?;
                super::state_io::write_atomic(
                    &path.with_extension("toml.pre-repair"),
                    original.as_bytes(),
                )?;
                crate::desk_log_warn!(
                    "config",
                    "Repaired legacy duplicate sections; original backed up at {}",
                    path.display()
                );
                doc
            }
        };
        change(&mut doc)?;
        let next = doc.to_string();
        if next == original {
            return Ok(false);
        }
        // Detect external edits during transformation. This is optimistic,
        // not a cross-process lock against a non-cooperating editor.
        let current = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("Cannot recheck configuration: {e}")),
        };
        if current != original {
            return Err("CONFIG_CONFLICT: configuration changed externally; retry".into());
        }
        super::state_io::write_atomic(path, next.as_bytes())?;
        Ok(true)
    })();
    match &result {
        Ok(changed) => crate::desk_log_debug!(
            "config",
            "edit={} changed={} path={}",
            operation,
            changed,
            path.display()
        ),
        Err(e) => crate::desk_log_error!(
            "config",
            "edit={} path={} failed: {}",
            operation,
            path.display(),
            e
        ),
    }
    result
}

pub fn set(
    doc: &mut DocumentMut,
    section: &str,
    key: &str,
    mut value: Value,
) -> Result<(), String> {
    if !doc.contains_key(section) {
        doc[section] = Item::Table(toml_edit::Table::new());
    }
    let table = doc[section]
        .as_table_mut()
        .ok_or_else(|| format!("Configuration section {section} is not a table"))?;
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        if old.to_string().trim() == value.to_string().trim() {
            return Ok(());
        }
        *value.decor_mut() = old.decor().clone();
    }
    table[key] = Item::Value(value);
    Ok(())
}
pub fn set_value(path: &Path, section: &str, key: &str, value: Value) -> Result<bool, String> {
    edit(path, key, |doc| set(doc, section, key, value))
}
pub fn ensure_value(path: &Path, section: &str, key: &str, value: Value) -> Result<bool, String> {
    edit(path, key, |doc| {
        if doc.get(section).and_then(|s| s.get(key)).is_none() {
            set(doc, section, key, value)?;
        }
        Ok(())
    })
}
pub fn read(path: &Path) -> Option<DocumentMut> {
    match fs::read_to_string(path) {
        Ok(text) => match text.parse() {
            Ok(doc) => Some(doc),
            Err(_) => {
                crate::desk_log_warn!("config", "Invalid TOML while reading {}", path.display());
                None
            }
        },
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                crate::desk_log_warn!("config", "Cannot read {}: {}", path.display(), e);
            }
            None
        }
    }
}

/// Initialization/migration is part of the SAME writer contract. Existing
/// documents win; never remove the source on a failed publication.
pub fn initialize(path: &Path, legacy: Option<&Path>, default: &str) -> Result<(), String> {
    let _guard = EDIT
        .lock()
        .map_err(|_| "Configuration editor unavailable")?;
    let _file_lock = super::state_io::lock(&path.with_extension("toml.lock"))?;
    match fs::metadata(path) {
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Cannot inspect configuration: {e}")),
    }
    let content = if let Some(legacy) = legacy.filter(|p| p.is_file()) {
        fs::read_to_string(legacy).map_err(|e| format!("Cannot read legacy configuration: {e}"))?
    } else {
        default.to_owned()
    };
    super::state_io::write_atomic(path, content.as_bytes())?;
    // Keep the legacy original; compatibility sync will update it explicitly.
    crate::desk_log_info!(
        "config",
        "Initialized configuration {} (legacy source retained)",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn section_scoped_and_escaped_preserving_comments() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        fs::write(
            &p,
            "# top\n[other]\nlevel = \"other\"\n[log] # comment\nlevel = \"info\" # keep\n",
        )
        .unwrap();
        set_value(&p, "log", "level", Value::from("debug")).unwrap();
        let name = "line\n\"quoted\"\\tab\t$1";
        set_value(&p, "presence", "custom_game_name", Value::from(name)).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        let doc = read(&p).unwrap();
        assert_eq!(doc["other"]["level"].as_str(), Some("other"));
        assert_eq!(doc["presence"]["custom_game_name"].as_str(), Some(name));
        assert!(text.contains("# keep"));
        assert!(text.contains("# top"));
    }
    #[test]
    fn corrupt_document_is_not_replaced() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.toml");
        fs::write(&p, "[broken").unwrap();
        assert!(set_value(&p, "log", "level", Value::from("info")).is_err());
        assert_eq!(fs::read_to_string(p).unwrap(), "[broken");
    }
    #[test]
    fn concurrent_edits_preserve_both_fields() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.toml");
        std::thread::scope(|s| {
            for key in ["first", "second"] {
                let p = &p;
                s.spawn(move || {
                    for _ in 0..20 {
                        set_value(p, "section", key, Value::from(true)).unwrap();
                    }
                });
            }
        });
        let doc = read(&p).unwrap();
        assert_eq!(doc["section"]["first"].as_bool(), Some(true));
        assert_eq!(doc["section"]["second"].as_bool(), Some(true));
    }
    #[test]
    fn duplicate_legacy_sections_are_backed_up_and_repaired() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.toml");
        let source = "[presence]\ndefault_mode = \"none\"\n[presence]\nexclude_apps = [12]\n";
        fs::write(&p, source).unwrap();
        set_value(&p, "presence", "custom_game_name", Value::from("name")).unwrap();
        let doc = read(&p).unwrap();
        assert_eq!(doc["presence"]["default_mode"].as_str(), Some("none"));
        assert_eq!(doc["presence"]["exclude_apps"].as_array().unwrap().len(), 1);
        assert_eq!(
            fs::read_to_string(p.with_extension("toml.pre-repair")).unwrap(),
            source
        );
    }
    #[test]
    fn initializer_never_overwrites_user_config() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.toml");
        initialize(&p, None, "[log]\nlevel = \"info\"\n").unwrap();
        set_value(&p, "log", "level", Value::from("debug")).unwrap();
        initialize(&p, None, "[log]\nlevel = \"error\"\n").unwrap();
        assert_eq!(read(&p).unwrap()["log"]["level"].as_str(), Some("debug"));
    }
}
