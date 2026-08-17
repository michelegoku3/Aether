//! Lettura/scrittura delle LaunchOptions di Steam in `localconfig.vdf`.
//!
//! Steam salva gli argomenti di avvio per gioco in:
//! `<Steam>/userdata/<user>/config/localconfig.vdf`, sezione
//! `"Apps"` -> `"<appid>"` -> `"LaunchOptions"`.
//!
//! Il file è VDF testuale; Steam lo riscrive quando esce, quindi le nostre
//! modifiche vengono preservate finché Steam è chiuso o le riscrive da sé.
//! Le funzioni qui lavorano sul testo preservando tutto il resto del file.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Cerca `localconfig.vdf` nella userdata di Steam: tra tutte le cartelle
/// utente, preferisce quella con il file modificato più di recente.
pub fn find_localconfig(steam_path: &Path) -> Option<PathBuf> {
    let userdata = steam_path.join("userdata");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(&userdata).ok()?.flatten() {
        let cfg = entry.path().join("config").join("localconfig.vdf");
        if !cfg.is_file() {
            continue;
        }
        let modified = fs::metadata(&cfg)
            .and_then(|meta| meta.modified())
            .unwrap_or(UNIX_EPOCH);
        if best.as_ref().map(|(best_time, _)| modified > *best_time).unwrap_or(true) {
            best = Some((modified, cfg));
        }
    }
    best.map(|(_, path)| path)
}

/// Legge le LaunchOptions attuali del gioco (stringa vuota se assenti).
pub fn get_launch_options(steam_path: &Path, app_id: u32) -> Result<String, String> {
    let path = find_localconfig(steam_path)
        .ok_or_else(|| "localconfig.vdf not found in Steam userdata.".to_string())?;
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    Ok(read_app_launch_options(&content, app_id))
}

/// Imposta le LaunchOptions del gioco (sostituisce l'intero valore).
pub fn set_launch_options(steam_path: &Path, app_id: u32, options: &str) -> Result<(), String> {
    let path = find_localconfig(steam_path)
        .ok_or_else(|| "localconfig.vdf not found in Steam userdata.".to_string())?;
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let updated = set_app_launch_options(&content, app_id, options)
        .ok_or_else(|| "Failed to update localconfig.vdf (unexpected structure).".to_string())?;

    // Backup prima di scrivere.
    let backup = path.with_extension("vdf.bak");
    let _ = fs::copy(&path, &backup);
    fs::write(&path, updated)
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// True quando le LaunchOptions contengono il token (es. "-onlinefix").
pub fn has_launch_token(options: &str, token: &str) -> bool {
    options.split_whitespace().any(|arg| arg.eq_ignore_ascii_case(token))
}

/// Aggiunge o rimuove un token dalle LaunchOptions preservando gli altri
/// argomenti. Ritorna le nuove LaunchOptions.
pub fn toggle_launch_token(options: &str, token: &str, enabled: bool) -> String {
    let args: Vec<&str> = options.split_whitespace().collect();
    let mut kept: Vec<&str> = Vec::with_capacity(args.len() + 1);
    for arg in args {
        if !arg.eq_ignore_ascii_case(token) {
            kept.push(arg);
        }
    }
    if enabled {
        kept.push(token);
    }
    kept.join(" ")
}

/// Escaping VDF per un valore di chiave: `\` e `"`.
fn vdf_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Indice della prima `"` non preceduta da un numero dispari di backslash.
fn find_unescaped_quote(s: &str, from: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut backslashes = 0;
            let mut j = i;
            while j > 0 && bytes[j - 1] == b'\\' {
                backslashes += 1;
                j -= 1;
            }
            if backslashes % 2 == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Posizione della riga `"<key>"` (seguita da `{` per i blocchi, o da un
/// valore per le chiavi) e della fine della chiave.
fn find_key(content: &str, key: &str) -> Option<(usize, usize)> {
    let mut search = 0;
    while let Some(rel) = content[search..].find(key) {
        let start = search + rel;
        let after = start + key.len();
        let rest = &content[after..];
        let trimmed = rest.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('"') {
            return Some((start, after));
        }
        search = start + 1;
    }
    None
}

/// Indice della `}` che chiude il blocco aperto da `{` in `open_idx`.
fn find_matching_brace(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    for i in open_idx..bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Legge le LaunchOptions del gioco dal testo VDF.
fn read_app_launch_options(content: &str, app_id: u32) -> String {
    let app_key = format!("\"{app_id}\"");
    let Some((_, apps_after)) = find_key(content, "\"Apps\"") else {
        return String::new();
    };
    let Some(apps_open_rel) = content[apps_after..].find('{') else {
        return String::new();
    };
    let apps_open = apps_after + apps_open_rel;
    let Some(apps_close) = find_matching_brace(content, apps_open) else {
        return String::new();
    };

    let apps_block = &content[apps_open + 1..apps_close];
    let Some((app_start, app_after)) = find_key(apps_block, &app_key) else {
        return String::new();
    };
    // Indici relativi ad apps_block: la `{` del blocco app sta dopo la chiave.
    let Some(open_rel) = apps_block[app_after..].find('{') else {
        return String::new();
    };
    let open = app_after + open_rel;
    let Some(close) = find_matching_brace(apps_block, open) else {
        return String::new();
    };
    let app_block = &apps_block[open + 1..close];

    if let Some((_, lo_after)) = find_key(app_block, "\"LaunchOptions\"") {
        if let Some(value_rel) = app_block[lo_after..].find('"') {
            let value_start = lo_after + value_rel + 1;
            if let Some(value_end) = find_unescaped_quote(app_block, value_start) {
                return app_block[value_start..value_end]
                    .replace("\\\\", "\\")
                    .replace("\\\"", "\"");
            }
        }
    }
    String::new()
}

/// Imposta le LaunchOptions del gioco nel testo VDF. Ritorna il nuovo testo.
fn set_app_launch_options(content: &str, app_id: u32, options: &str) -> Option<String> {
    let app_key = format!("\"{app_id}\"");
    let (_, apps_after) = find_key(content, "\"Apps\"")?;
    let apps_open_rel = content[apps_after..].find('{')?;
    let apps_open = apps_after + apps_open_rel;
    let apps_close = find_matching_brace(content, apps_open)?;

    let escaped = vdf_escape(options);

    if let Some((app_start, app_after)) = find_key(&content[apps_open + 1..apps_close], &app_key) {
        let abs_app_start = apps_open + 1 + app_start;
        // Trova il blocco `{...}` dell'app.
        let block = &content[abs_app_start..apps_close];
        let open_rel = block[app_after - app_start..].find('{')?;
        let open = app_after - app_start + open_rel;
        let close = find_matching_brace(block, open)?;
        let abs_open = abs_app_start + open;
        let abs_close = abs_app_start + close;

        let app_block = &content[abs_open + 1..abs_close];
        if let Some((lo_start, lo_after)) = find_key(app_block, "\"LaunchOptions\"") {
            // Sostituisce il valore esistente.
            let abs_lo_start = abs_open + 1 + lo_start;
            let value_rel = app_block[lo_after..].find('"')?;
            let value_start = abs_lo_start + lo_after - lo_start + value_rel + 1;
            let value_end_abs = value_start + find_unescaped_quote(&content[value_start..abs_close], 0)?;
            let mut updated = String::with_capacity(content.len() + escaped.len());
            updated.push_str(&content[..value_start]);
            updated.push_str(&escaped);
            updated.push_str(&content[value_end_abs..]);
            return Some(updated);
        }

        // Nessuna LaunchOptions nel blocco app: inserisce subito dopo `{`.
        let insert = format!("\t\"LaunchOptions\"\t\t\"{escaped}\"\n");
        let mut updated = String::with_capacity(content.len() + insert.len());
        updated.push_str(&content[..abs_open + 1]);
        updated.push('\n');
        updated.push_str(&insert);
        updated.push_str(&content[abs_open + 1..]);
        return Some(updated);
    }

    // Il blocco app non esiste: lo crea dentro `Apps`, prima della `}`.
    let insert = format!(
        "\n\t\"{app_id}\"\n\t{{\n\t\t\"LaunchOptions\"\t\t\"{escaped}\"\n\t}}\n"
    );
    let mut updated = String::with_capacity(content.len() + insert.len());
    updated.push_str(&content[..apps_close]);
    updated.push_str(&insert);
    updated.push_str(&content[apps_close..]);
    Some(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_vdf() -> String {
        r#""UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"Apps"
				{
					"730"
					{
						"LaunchOptions"		"-novid"
					}
				}
			}
		}
	}
}
"#
        .to_string()
    }

    #[test]
    fn reads_existing_launch_options() {
        assert_eq!(read_app_launch_options(&sample_vdf(), 730), "-novid");
    }

    #[test]
    fn reads_empty_when_app_missing() {
        assert_eq!(read_app_launch_options(&sample_vdf(), 480), "");
    }

    #[test]
    fn replaces_existing_value() {
        let updated = set_app_launch_options(&sample_vdf(), 730, "-novid -onlinefix").unwrap();
        assert_eq!(read_app_launch_options(&updated, 730), "-novid -onlinefix");
        // Il resto del file è intatto.
        assert!(updated.contains("\"UserLocalConfigStore\""));
        assert!(updated.contains("\"Steam\""));
    }

    #[test]
    fn adds_key_to_existing_app_block() {
        let updated = set_app_launch_options(&sample_vdf(), 440, "-onlinefix").unwrap();
        assert_eq!(read_app_launch_options(&updated, 440), "-onlinefix");
        assert_eq!(read_app_launch_options(&updated, 730), "-novid");
    }

    #[test]
    fn creates_missing_app_block() {
        let updated = set_app_launch_options(&sample_vdf(), 1144200, "-onlinefix").unwrap();
        assert_eq!(read_app_launch_options(&updated, 1144200), "-onlinefix");
        assert_eq!(read_app_launch_options(&updated, 730), "-novid");
    }

    #[test]
    fn toggles_token() {
        assert_eq!(toggle_launch_token("-novid", "-onlinefix", true), "-novid -onlinefix");
        assert_eq!(
            toggle_launch_token("-novid -onlinefix", "-onlinefix", false),
            "-novid"
        );
        assert!(has_launch_token("-novid -ONLINEFIX", "-onlinefix"));
        assert!(!has_launch_token("-novid", "-onlinefix"));
    }

    #[test]
    fn escapes_special_characters() {
        let updated = set_app_launch_options(&sample_vdf(), 730, r#"C:\path "with quotes""#).unwrap();
        assert_eq!(read_app_launch_options(&updated, 730), r#"C:\path "with quotes""#);
    }
}
