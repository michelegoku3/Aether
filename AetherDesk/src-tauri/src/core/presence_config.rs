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

use crate::core::settings::SettingsManager;

/// Le copie di aethercore.toml aggiornate da AetherDesk (stesso dual-path
/// già usato per custom_game_name in commands/settings.rs).
pub fn aethercore_toml_paths(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    let mut paths = vec![crate::core::paths::LocalAppPaths::config_dir().join("aethercore.toml")];
    let steam_path = SettingsManager::new(app).load().steam_path;
    if !steam_path.trim().is_empty() {
        paths.push(std::path::PathBuf::from(&steam_path).join("aethercore").join("aethercore.toml"));
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

pub fn parse_appid_list(text: &str) -> Vec<u32> {
    text.trim()
        .trim_start_matches('[')
        .split_terminator(|c: char| c == ',' || c == ']')
        .filter_map(|tok| tok.trim().parse::<u32>().ok())
        .collect()
}

pub fn format_appid_list(apps: &[u32]) -> String {
    format!(
        "[{}]",
        apps.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
    )
}

/// (indice riga, contenuto) per la prima riga NON commentata `key = ...`.
pub fn find_key_line(lines: &[String], key: &str) -> Option<usize> {
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix(key) {
            if rest.trim_start().starts_with('=') {
                return Some(i);
            }
        }
    }
    None
}

pub fn read_mode_apps(path: &std::path::Path, mode: PresenceMode) -> Option<Vec<u32>> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    let idx = find_key_line(&lines, mode.key())?;
    let after_eq = lines[idx].splitn(2, '=').nth(1)?;
    Some(parse_appid_list(after_eq))
}

pub fn read_default_mode(path: &std::path::Path) -> Option<bool> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    let idx = find_key_line(&lines, "default_mode")?;
    let after_eq = lines[idx].splitn(2, '=').nth(1)?;
    Some(after_eq.trim().trim_matches('"') == "showonline")
}

/// Sostituisce la prima riga non commentata `key = value` oppure la inserisce
/// nella posizione `insert_at` (stabilita dal chiamante: subito sotto
/// l'header di [presence], avanzando ad ogni inserimento).
pub fn upsert_key_line(lines: &mut Vec<String>, key: &str, value: &str, insert_at: &mut usize) {
    let new_line = format!("{key} = {value}");
    if let Some(i) = find_key_line(lines, key) {
        lines[i] = new_line;
    } else {
        lines.insert(*insert_at, new_line);
        *insert_at += 1;
    }
}

/// Aggiunge/rimuove app_id dagli array [presence] del toml indicato:
/// `choice = Some(mode)` → app nella lista di `mode` e fuori dalle altre due
/// (mutua esclusione); `choice = None` → fuori da tutte (torna al default).
/// Le tre righe sono (ri)scritte in forma canonica; [presence] e le chiavi
/// mancanti vengono create. Ritorna true se il file è stato modificato.
pub fn update_mode_in_toml(path: &std::path::Path, app_id: u32, choice: Option<PresenceMode>) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let had_trailing_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if lines.len() == 1 && lines[0].is_empty() {
        lines.clear();
    }

    let mut lists: [Vec<u32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut key_present: [bool; 3] = [false, false, false];
    let mut presence_hdr: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue;
        }
        if t == "[presence]" {
            presence_hdr = Some(i);
        }
        for (m, mode) in PresenceMode::ALL.iter().enumerate() {
            if key_present[m] {
                continue;
            }
            if let Some(rest) = t.strip_prefix(mode.key()) {
                let rest = rest.trim_start();
                if let Some(list) = rest.strip_prefix('=') {
                    key_present[m] = true;
                    lists[m] = parse_appid_list(list);
                }
            }
        }
    }

    let original = lists.clone();
    for list in lists.iter_mut() {
        list.retain(|&a| a != app_id);
    }
    if let Some(c) = choice {
        let m = PresenceMode::ALL.iter().position(|&x| x == c).expect("mode in ALL");
        lists[m].push(app_id);
        lists[m].sort_unstable();
        lists[m].dedup();
    }
    if lists == original && key_present.iter().all(|&p| p) {
        return false; // già nello stato richiesto, niente da riscrivere
    }

    // Assicura la sezione [presence] (append in coda se assente).
    let hdr = match presence_hdr {
        Some(h) => h,
        None => {
            lines.push("[presence]".to_string());
            lines.len() - 1
        }
    };
    let mut insertion = hdr + 1;
    for (m, mode) in PresenceMode::ALL.iter().enumerate() {
        upsert_key_line(&mut lines, mode.key(), &format_appid_list(&lists[m]), &mut insertion);
    }

    let mut out = lines.join("\n");
    if had_trailing_nl || !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(path, out).is_ok()
}

/// Scrive `default_mode = "showonline"|"none"` sotto [presence].
pub fn set_default_mode_in_toml(path: &std::path::Path, showonline: bool) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let had_trailing_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if lines.len() == 1 && lines[0].is_empty() {
        lines.clear();
    }
    let value = if showonline { "\"showonline\"" } else { "\"none\"" };
    if let Some(i) = find_key_line(&lines, "default_mode") {
        if lines[i].splitn(2, '=').nth(1).map(|v| v.trim()) == Some(value) {
            return false; // già impostato
        }
        lines[i] = format!("default_mode = {value}");
    } else {
        let hdr = lines
            .iter()
            .position(|l| l.trim_start() == "[presence]");
        match hdr {
            Some(h) => lines.insert(h + 1, format!("default_mode = {value}")),
            None => {
                lines.push("[presence]".to_string());
                lines.push(format!("default_mode = {value}"));
            }
        }
    }
    let mut out = lines.join("\n");
    if had_trailing_nl || !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(path, out).is_ok()
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
    t.split('=').next().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
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
        let known: Vec<String> = (first + 1..first_end).filter_map(|i| toml_key_of(&lines[i])).collect();
        let mut merged: Vec<String> = Vec::new();
        for &dup in &idxs[1..] {
            let dup_end = next_hdr(dup + 1);
            for i in dup..dup_end {
                if let Some(k) = toml_key_of(&lines[i]) {
                    if !known.contains(&k) && !merged.iter().any(|m| toml_key_of(m).as_deref() == Some(k.as_str())) {
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

/// Inserisce le chiavi `[presence]` canoniche MANCANTI senza toccare quelle
/// esistenti (usato dal bridge di migrazione ad ogni avvio Desk; la stessa
/// policy entra nel default bundlato per le nuove installazioni).
/// `showonline` come default_mode base: presenza globale opt-out (§13).
pub fn ensure_defaults(path: &Path) {
    if !path.exists() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let had_trailing_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if lines.len() == 1 && lines[0].is_empty() {
        lines.clear();
    }

    // Prima di ogni altra operazione: ripara sezioni duplicate (una copia
    // bundlata di aethercore.toml finita su disco in passato dichiarava
    // [presence] due volte → file INVALIDO per TOML 1.0 → la DLL azzerava
    // silenziosamente tutta la config ai default: livello log Warn e liste
    // presenza vuote). Senza questa riparazione i file già danneggiati sul
    // disco resterebbero inutilizzabili per sempre.
    let mut changed = dedup_sections(&mut lines);

    let hdr = match lines.iter().position(|l| l.trim_start() == "[presence]") {
        Some(h) => h,
        None => {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push("[presence]".to_string());
            lines.len() - 1
        }
    };
    if hdr == lines.len() - 1 && !lines.is_empty() && lines.last().map(|l| l == "[presence]").unwrap_or(false) {
        changed = true; // sezione creata ora
    }

    let mut insertion = hdr + 1;
    let wanted: [(&str, String); 4] = [
        ("default_mode", "\"showonline\"".to_string()),
        (PresenceMode::ShowOnline.key(), format_appid_list(&[])),
        (PresenceMode::AetherOnline.key(), format_appid_list(&[])),
        (PresenceMode::Excluded.key(), format_appid_list(&[])),
    ];
    for (key, value) in wanted {
        if find_key_line(&lines, key).is_none() {
            lines.insert(insertion, format!("{key} = {value}"));
            insertion += 1;
            changed = true;
        }
    }
    if !changed {
        return;
    }
    let mut out = lines.join("\n");
    if had_trailing_nl || !out.ends_with('\n') {
        out.push('\n');
    }
    let _ = std::fs::write(path, out);
}

/// Legacy key rename (pre-rename installs): `onlinefix_apps` ->
/// `aetheronline_apps`, `onlinefix_persona_patch` -> `aetheronline_persona_patch`.
/// Line-based and comment-safe: only the FIRST non-commented occurrence is
/// renamed, and ONLY when the new key is not present yet. Idempotent — after
/// the first migration the legacy line no longer exists.
pub fn migrate_legacy_presence_keys(path: &Path) {
    if !path.exists() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let had_trailing_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let mut changed = false;
    for (legacy, new) in [
        ("onlinefix_apps", "aetheronline_apps"),
        ("onlinefix_persona_patch", "aetheronline_persona_patch"),
    ] {
        if find_key_line(&lines, new).is_some() {
            continue; // already migrated (or hand-written new key)
        }
        if let Some(i) = find_key_line(&lines, legacy) {
            let line = &lines[i];
            let Some(after_eq) = line.splitn(2, '=').nth(1) else {
                continue;
            };
            lines[i] = format!("{new} = {}", after_eq);
            changed = true;
        }
    }
    if !changed {
        return;
    }
    let mut out = lines.join("\n");
    if had_trailing_nl || !out.ends_with('\n') {
        out.push('\n');
    }
    let _ = std::fs::write(path, out);
}
