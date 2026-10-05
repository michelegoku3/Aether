//! Centralised per-app launch policy persistence in `aethercore.toml` (docs/05
//! §11-§13 — shared by the tauri commands and the Desk migration layer.
//!
//! Layout gestito dal modulo:
//!
//! ```toml
//! [presence]
//! default_mode    = "none"      # "none" | "showonline" (policy + overrides)
//! showonline_apps = [...]
//! aetheronline_apps  = [...]
//! exclude_apps    = [...]       # hard opt-out: vince su token e array
//! ```
//!
//! La DLL risolve UNA modalità per app dentro SpawnProcess rileggendo questo
//! file (mtime → nessun riavvio di Steam); nulla finisce in argv. Precedenza
//! documentata: exclude > aetheronline > showonline > default_mode. I token
//! `-aetheronline` / `-showonline` nelle LaunchOptions sono LEGACY: rimossi dai
//! set (migrazione) e comunque riconosciuti dai get finché non migrati.
//!
//! Line-based editing intenzionale: il file resta hand-editable, commenti e
//! sezioni estranee sono preservati byte-per-line.

use std::path::Path;

use crate::core::settings::load_settings;

/// Le copie di aethercore.toml aggiornate da AetherDesk (stesso dual-path
/// già usato per custom_game_name in commands/settings.rs).
pub fn aethercore_toml_paths(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    let mut paths = vec![crate::core::paths::LocalAppPaths::config_dir().join("aethercore.toml")];
    let steam_path = load_settings(app).steam_path;
    if !steam_path.trim().is_empty() {
        let legacy = std::path::PathBuf::from(&steam_path)
            .join("aethercore")
            .join("aethercore.toml");
        if legacy.exists() && !paths.contains(&legacy) {
            paths.push(legacy);
        }
    }
    paths
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PresenceMode {
    ShowOnline,
    AetherOnline,
    Excluded,
}

impl PresenceMode {
    fn key(self) -> &'static str {
        match self {
            PresenceMode::ShowOnline => "showonline_apps",
            PresenceMode::AetherOnline => "aetheronline_apps",
            PresenceMode::Excluded => "exclude_apps",
        }
    }
    const ALL: [PresenceMode; 3] = [
        PresenceMode::ShowOnline,
        PresenceMode::AetherOnline,
        PresenceMode::Excluded,
    ];
}

pub fn read_mode_apps(path: &Path, mode: PresenceMode) -> Option<Vec<u32>> {
    let doc = super::config_document::read(path)?;
    Some(
        doc.get("presence")?
            .get(mode.key())?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_integer().and_then(|n| u32::try_from(n).ok()))
            .collect(),
    )
}
pub fn read_default_mode(path: &Path) -> Option<bool> {
    let doc = super::config_document::read(path)?;
    Some(doc.get("presence")?.get("default_mode")?.as_str()? == "showonline")
}
pub fn update_mode_in_toml(
    path: &Path,
    app_id: u32,
    choice: Option<PresenceMode>,
) -> Result<bool, String> {
    super::config_document::edit(path, "presence-app", |doc| {
        for mode in PresenceMode::ALL {
            let mut apps: Vec<u32> = doc
                .get("presence")
                .and_then(|s| s.get(mode.key()))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_integer().and_then(|n| u32::try_from(n).ok()))
                        .collect()
                })
                .unwrap_or_default();
            apps.retain(|&id| id != app_id);
            if choice == Some(mode) {
                apps.push(app_id);
            }
            apps.sort_unstable();
            apps.dedup();
            let mut array = toml_edit::Array::new();
            for id in apps {
                array.push(i64::from(id));
            }
            super::config_document::set(
                doc,
                "presence",
                mode.key(),
                toml_edit::Value::Array(array),
            )?;
        }
        Ok(())
    })
}
pub fn set_default_mode_in_toml(path: &Path, show_online: bool) -> Result<bool, String> {
    super::config_document::set_value(
        path,
        "presence",
        "default_mode",
        toml_edit::Value::from(if show_online { "showonline" } else { "none" }),
    )
}

/// Header di sezione TOML (`[x]`, esclusi gli array-of-tables `[[x]]` che
/// possono legittimamente ripetersi). Restituisce il nome tra le parentesi.
/// fn e non closure: l'elision lifetime delle fn lega il riferimento
/// restituito all'input, quella delle closure no (errore '1 vs '2).
fn section_header_name(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.starts_with('[') && t.ends_with(']') && !t.starts_with("[[") {
        Some(&t[1..t.len() - 1])
    } else {
        None
    }
}

/// Nome chiave di una riga `key = value`; None per commenti, header e righe
/// non-assegnazione.
fn toml_key_of(s: &str) -> Option<String> {
    let t = s.trim_start();
    if t.starts_with('#') || section_header_name(t).is_some() {
        return None;
    }
    t.split('=')
        .next()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// Rimuove sezioni TOML duplicate (`[x]` dichiarato due volte): TOML 1.0 lo
/// vieta, quindi una sola sezione doppia rende INVALIDO l'intero file e i
/// parser reali (toml++) azzerano TUTTO ai default. Merge conservativo: le
/// righe `key = value` del duplicato vengono spostate in coda alla prima
/// occorrenza della stessa sezione, saltando le chiavi già presenti lì (vince
/// sempre la prima occorrenza = scritta per prima). Commenti e righe vuote del
/// blocco duplicato vengono eliminati. Restituisce true se ha modificato.
pub fn dedup_sections(lines: &mut Vec<String>) -> bool {
    // Mappa sezione -> indici di tutte le sue occorrenze.
    let mut occurrences: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if let Some(name) = section_header_name(l) {
            match occurrences.iter_mut().find(|(n, _)| n == name) {
                Some((_, v)) => v.push(i),
                None => occurrences.push((name.to_string(), vec![i])),
            }
        }
    }
    if occurrences.iter().all(|(_, v)| v.len() == 1) {
        return false;
    }

    // Per ogni sezione duplicata: estrai le chiavi uniche dai blocchi extra e
    // marca per la cancellazione tutte le righe di quei blocchi.
    let mut dropped = vec![false; lines.len()];
    // (posizione di inserzione in coda alla prima sezione, chiavi da fondere)
    let mut merge_at: Vec<(usize, Vec<String>)> = Vec::new();
    for (_, idxs) in &occurrences {
        if idxs.len() < 2 {
            continue;
        }
        let first = idxs[0];
        // Chiavi già presenti nella prima occorrenza.
        let next_hdr = |from: usize| -> usize {
            let mut j = from;
            while j < lines.len() && section_header_name(&lines[j]).is_none() {
                j += 1;
            }
            j
        };
        let first_end = next_hdr(first + 1);
        let known: Vec<String> = (first + 1..first_end)
            .filter_map(|i| toml_key_of(&lines[i]))
            .collect();
        let mut merged: Vec<String> = Vec::new();
        for &dup in &idxs[1..] {
            let dup_end = next_hdr(dup + 1);
            for i in dup..dup_end {
                if let Some(k) = toml_key_of(&lines[i]) {
                    if !known.contains(&k)
                        && !merged
                            .iter()
                            .any(|m| toml_key_of(m).as_deref() == Some(k.as_str()))
                    {
                        merged.push(lines[i].trim().to_string());
                    }
                }
                dropped[i] = true;
            }
        }
        if !merged.is_empty() {
            merge_at.push((first_end, merged));
        }
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    for (i, l) in lines.iter().enumerate() {
        for (pos, merged) in &merge_at {
            if *pos == i {
                out.extend(merged.iter().cloned());
            }
        }
        if !dropped[i] {
            out.push(l.clone());
        }
    }
    // Chiusura file: se la prima sezione arrivava a EOF, il merge va in coda.
    for (pos, merged) in &merge_at {
        if *pos == lines.len() {
            out.extend(merged.iter().cloned());
        }
    }
    *lines = out;
    true
}

pub fn ensure_defaults(path: &Path) {
    if !path.exists() {
        return;
    }
    let _ = super::config_document::edit(path, "presence-defaults", |doc| {
        for (key, value) in [
            ("default_mode", toml_edit::Value::from("showonline")),
            (
                "showonline_apps",
                toml_edit::Value::Array(toml_edit::Array::new()),
            ),
            (
                "aetheronline_apps",
                toml_edit::Value::Array(toml_edit::Array::new()),
            ),
            (
                "exclude_apps",
                toml_edit::Value::Array(toml_edit::Array::new()),
            ),
        ] {
            if doc.get("presence").and_then(|s| s.get(key)).is_none() {
                super::config_document::set(doc, "presence", key, value)?;
            }
        }
        Ok(())
    }); // Startup best-effort; editor logs every failure.
}
pub fn migrate_legacy_presence_keys(path: &Path) {
    if !path.exists() {
        return;
    }
    let _ = super::config_document::edit(path, "presence-legacy-keys", |doc| {
        if let Some(table) = doc.get_mut("presence").and_then(|s| s.as_table_mut()) {
            for (old, new) in [
                ("onlinefix_apps", "aetheronline_apps"),
                ("onlinefix_persona_patch", "aetheronline_persona_patch"),
            ] {
                if !table.contains_key(new) {
                    if let Some(value) = table.remove(old) {
                        table.insert(new, value);
                    }
                }
            }
        }
        Ok(())
    });
}
