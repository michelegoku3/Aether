//! Contratto IPC Desk <-> frontend, verificato sul sorgente reale.
//!
//! # Perché questo file esiste
//!
//! Tauri 2 genera la chiave di ogni argomento di comando con
//! `param.to_lower_camel_case()` e la cerca nel payload con ESATTAMENTE quella
//! chiave (`tauri 2.11.5`, `crates/tauri/src/ipc/command.rs`: `v.get(self.key)`).
//! Non esiste alcun fallback snake_case. Una chiave sbagliata lato frontend non
//! è quindi un errore di compilazione: è una Promise rigettata a runtime con
//! `command X missing required key Y`.
//!
//! Il caso reale che ha motivato questi test: `set_presence_default_mode`
//! dichiarava `showonline: bool`, parola composta tutta minuscola copiata dal
//! token di dominio `default_mode = "showonline"` di `aethercore.toml`. La
//! chiave sul filo era quindi `showonline` e il frontend mandava `{ showonline }`:
//! funzionava per coincidenza. Un rename "di pulizia" fatto da una sola delle due
//! parti avrebbe rotto il toggle di presenza senza che `tsc` o `cargo check`
//! dicessero nulla.
//!
//! # Cosa viene verificato
//!
//! 1. ogni comando registrato in `generate_handler!` esiste nel layer comandi;
//! 2. ogni `invoke('…')` del frontend chiama un comando registrato;
//! 3. le chiavi degli argomenti del frontend coincidono con le chiavi wire
//!    derivate dalle firme Rust, e le chiavi obbligatorie sono tutte presenti;
//! 4. nessuna chiave wire è una "parola composta minuscola" (la classe di bug
//!    di cui sopra), nessuna chiave è un snake_case non convertito;
//! 5. nessun comando riceve più il percorso Steam dal client;
//! 6. i comandi citati nei contratti documentati hanno esattamente le chiavi
//!    attese (regressione puntuale sui casi già accaduti);
//! 7. l'insieme dei comandi mai chiamati dal frontend è solo quello documentato.
//!
//! I test leggono i file sorgente invece di usare macro/derive perché il
//! contratto da proteggere attraversa due linguaggi: l'unico punto in cui le due
//! metà si incontrano è il testo che finisce nel payload.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Percorsi
// ---------------------------------------------------------------------------

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `AetherDesk/`: la radice che contiene sia `src/` (frontend) sia `src-tauri/`.
fn desk_dir() -> PathBuf {
    crate_dir()
        .parent()
        .expect("src-tauri/ deve stare dentro AetherDesk/")
        .to_path_buf()
}

fn frontend_dir() -> PathBuf {
    desk_dir().join("src")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("impossibile leggere {}: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// Modello del comando Rust
// ---------------------------------------------------------------------------

/// Tipi iniettati dal macro di Tauri: non arrivano dal payload, quindi non
/// hanno una chiave wire. Riconosciuti dal tipo (non dal nome) perché il nome
/// è libero: `enable_online(app_id, request: OnlineEnableRequest)` ha un
/// parametro wire che si chiama proprio `request`.
const INJECTED_TYPE_MARKERS: &[&str] = &[
    "AppHandle",
    "tauri::Window",
    "tauri::Webview",
    "Window<",
    "Webview<",
    "WebviewWindow",
    "tauri::State",
    "State<",
    "tauri::ipc::Invoke",
    "CommandArg",
    "IpcResponse",
    "Resolver",
];

/// Nomi convenzionali usati nel codebase per gli handle iniettati.
const INJECTED_PARAM_NAMES: &[&str] = &["app", "window", "webview", "state"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct WireParam {
    /// Nome del parametro in Rust (`app_id`).
    rust_name: String,
    /// Chiave attesa nel payload IPC (`appId`).
    wire_key: String,
    /// Tipo dichiarato, per distinguere gli `Option<_>` (chiave assente = None).
    rust_type: String,
}

#[derive(Debug, Clone)]
struct CommandSig {
    /// `library` per `src/commands/library.rs`.
    module: String,
    name: String,
    params: Vec<WireParam>,
    line: usize,
}

impl CommandSig {
    fn required_keys(&self) -> Vec<&str> {
        self.params
            .iter()
            .filter(|param| !param.rust_type.starts_with("Option<"))
            .map(|param| param.wire_key.as_str())
            .collect()
    }

    fn all_keys(&self) -> Vec<&str> {
        self.params
            .iter()
            .map(|param| param.wire_key.as_str())
            .collect()
    }
}

/// `app_id` -> `appId`.
///
/// Replica di `heck::AsLowerCamelCase` per l'unico alfabeto che il codebase usa
/// nei parametri (snake_case ASCII): [`test_param_names_stay_inside_the_model`]
/// fallisce se qualcuno introduce un nome con maiuscole o cifre iniziali, cioè
/// l'unico caso in cui questa funzione potrebbe divergere da heck.
fn wire_key(param: &str) -> String {
    let mut words = param.split('_').filter(|word| !word.is_empty());
    let Some(first) = words.next() else {
        return String::new();
    };
    let mut out = first.to_ascii_lowercase();
    for word in words {
        let mut chars = word.chars();
        if let Some(head) = chars.next() {
            out.push(head.to_ascii_uppercase());
            out.extend(chars.map(|ch| ch.to_ascii_lowercase()));
        }
    }
    out
}

/// Estrae il contenuto tra la prima `(` (o `[`, `{`) dopo `from` e la sua
/// chiusura, rispettando la nidificazione. Restituisce `(interno, indice_dopo)`.
fn balanced<'a>(source: &'a str, from: usize, open: char, close: char) -> Option<(&'a str, usize)> {
    let start = source[from..].find(open)? + from;
    let mut depth = 0usize;
    for (offset, ch) in source[start..].char_indices() {
        match ch {
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + 1;
                    return Some((&source[start + 1..end - 1], end));
                }
            }
            _ => {}
        }
    }
    None
}

/// Divide una lista di parametri (Rust) o di membri (oggetto TS) sulle virgole
/// di primo livello: ignora le virgole dentro `()`, `[]`, `{}`, `<>`.
fn split_top_level(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in input.chars() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

fn split_params(raw: &str) -> Vec<(String, String)> {
    split_top_level(raw)
        .into_iter()
        .filter_map(|param| {
            let param = param.trim();
            let (name, ty) = param.split_once(':')?;
            Some((
                name.trim().trim_start_matches("mut").trim().to_string(),
                ty.trim().to_string(),
            ))
        })
        .collect()
}

/// Firma di ogni `#[tauri::command]` del layer comandi.
fn command_signatures() -> BTreeMap<String, CommandSig> {
    let dir = crate_dir().join("src").join("commands");
    let mut signatures = BTreeMap::new();
    for entry in fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("manca {}: {error}", dir.display()))
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let module = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let source = read(&path);
        for marker in source.match_indices("#[tauri::command") {
            // Dal marker alla prima `fn` pubblica: in mezzo possono stare altri
            // attributi (#[allow], doc comment) ma mai un'altra firma.
            let Some(fn_at) = source[marker.0..].find("fn ") else {
                continue;
            };
            let fn_at = marker.0 + fn_at;
            let after_fn = fn_at + "fn ".len();
            let name_end = source[after_fn..]
                .find('(')
                .map(|offset| after_fn + offset)
                .unwrap_or(source.len());
            let name = source[after_fn..name_end].trim().to_string();
            let Some((raw_params, _)) = balanced(&source, name_end, '(', ')') else {
                continue;
            };
            let params = split_params(raw_params)
                .into_iter()
                .filter(|(name, ty)| {
                    !INJECTED_PARAM_NAMES.contains(&name.as_str())
                        && !INJECTED_TYPE_MARKERS.iter().any(|marker| ty.contains(marker))
                })
                .map(|(rust_name, rust_type)| WireParam {
                    wire_key: wire_key(rust_name.trim_start_matches('_')),
                    rust_name,
                    rust_type,
                })
                .collect();
            let line = source[..marker.0].matches('\n').count() + 1;
            let signature = CommandSig {
                module: module.clone(),
                name: name.clone(),
                params,
                line,
            };
            let previous = signatures.insert(name.clone(), signature);
            let previous_module = previous
                .as_ref()
                .map(|signature| signature.module.clone())
                .unwrap_or_else(|| "?".to_string());
            let previous_line = previous.as_ref().map(|signature| signature.line).unwrap_or(0);
            assert!(
                previous.is_none(),
                "comando duplicato `{name}`: commands/{module}.rs:{line} e                  commands/{previous_module}.rs:{previous_line}. Due #[tauri::command] con lo                  stesso nome rendono ambigua la registrazione in generate_handler!"
            );
        }
    }
    signatures
}

/// Comandi registrati in `generate_handler![…]` (`src/main.rs`), come
/// `(modulo, nome)`.
fn registered_commands() -> Vec<(String, String)> {
    let source = read(&crate_dir().join("src").join("main.rs"));
    let start = source
        .find("generate_handler!")
        .expect("src/main.rs deve registrare i comandi con generate_handler!");
    let (body, _) = balanced(&source, start, '[', ']').expect("generate_handler![…] non bilanciato");
    let mut out = Vec::new();
    for token in body.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let segments: Vec<&str> = token.split("::").map(str::trim).collect();
        match segments.as_slice() {
            ["commands", module, name] => out.push((module.to_string(), name.to_string())),
            _ => panic!(
                "voce inattesa in generate_handler!: `{token}` (atteso commands::<modulo>::<nome>)"
            ),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Modello delle chiamate frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FrontendCall {
    file: String,
    line: usize,
    command: String,
    /// Chiavi dell'oggetto payload; `None` = payload non letterale (variabile,
    /// spread, costruzione dinamica): la chiamata non è verificabile qui.
    keys: Option<Vec<String>>,
}

fn frontend_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
                continue;
            }
            match path.extension().and_then(|ext| ext.to_str()) {
                Some("ts") | Some("tsx") => out.push(path),
                _ => {}
            }
        }
    }
    let mut files = Vec::new();
    walk(&frontend_dir(), &mut files);
    files.sort();
    files
}

/// Chiavi di primo livello di un object literal TS.
///
/// Copre le forme usate nel codebase: `key: value`, shorthand `key`,
/// `'key': value`, chiave computata/spread (che rendono il payload non
/// verificabile → `None`).
fn object_keys(raw: &str) -> Option<Vec<String>> {
    let mut keys = Vec::new();
    for member in split_top_level(raw) {
        let member = member.trim();
        if member.is_empty() {
            continue;
        }
        if member.starts_with("...") || member.starts_with('[') {
            return None;
        }
        let key = match member.split_once(':') {
            Some((key, _)) => key.trim(),
            None => member,
        };
        let key = key.trim_matches(|ch| ch == '\'' || ch == '"').trim();
        if key.is_empty() {
            return None;
        }
        keys.push(key.to_string());
    }
    Some(keys)
}

/// Tutte le chiamate del frontend verso il backend: `invoke('…')` dirette e
/// `queryGameState(appId, '…')` (che aggiunge `appId` al payload da sola).
fn frontend_calls() -> Vec<FrontendCall> {
    let mut calls = Vec::new();
    for path in frontend_files() {
        let source = read(&path);
        let relative = path
            .strip_prefix(&desk_dir())
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        // --- invoke<T>('name', payload) / invoke('name', payload) -----------
        for (index, _) in source.match_indices("invoke") {
            if index > 0 {
                let previous = source[..index].chars().next_back().unwrap();
                if previous.is_ascii_alphanumeric() || previous == '_' || previous == '.' {
                    continue; // `reinvoke`, `foo.invoke`: non è la funzione IPC
                }
            }
            let after = index + "invoke".len();
            // `invoke<…>(` oppure `invoke(`: salta l'eventuale type argument.
            let paren_at = match source[after..].chars().next() {
                Some('<') => match balanced(&source, after, '<', '>') {
                    Some((_, end)) => end,
                    None => continue,
                },
                Some('(') => after,
                _ => continue,
            };
            let Some((args, _)) = balanced(&source, paren_at, '(', ')') else {
                continue;
            };
            let args = args.trim();
            let Some(command) = string_literal_arg(args) else {
                continue;
            };
            let keys = payload_keys(args, &source);
            calls.push(FrontendCall {
                line: source[..index].matches('\n').count() + 1,
                file: relative.clone(),
                command,
                keys,
            });
        }

        // --- queryGameState<T>(appId, 'name', args?) ------------------------
        for (index, _) in source.match_indices("queryGameState") {
            let Some(paren_at) = source[index..].find('(').map(|offset| index + offset) else {
                continue;
            };
            let Some((args, _)) = balanced(&source, paren_at, '(', ')') else {
                continue;
            };
            let parts = split_top_level(args);
            let Some(command) = parts.get(1).and_then(|raw| string_literal(raw.trim())) else {
                continue;
            };
            // Il provider aggiunge sempre `appId`; il terzo argomento (se c'è)
            // è l'oggetto con le chiavi restanti.
            let mut keys = vec!["appId".to_string()];
            let mut unverifiable = false;
            if let Some(extra) = parts.get(2) {
                match payload_argument_keys(extra, &source) {
                    Some(extra_keys) => keys.extend(extra_keys),
                    None => unverifiable = true,
                }
            }
            if unverifiable {
                calls.push(FrontendCall {
                    line: source[..index].matches('\n').count() + 1,
                    file: relative.clone(),
                    command,
                    keys: None,
                });
                continue;
            }
            calls.push(FrontendCall {
                line: source[..index].matches('\n').count() + 1,
                file: relative.clone(),
                command,
                keys: Some(keys),
            });
        }
    }
    calls
}

fn string_literal(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let inner = raw
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .or_else(|| raw.strip_prefix('"').and_then(|inner| inner.strip_suffix('"')))?;
    Some(inner.to_string())
}

/// Primo argomento, se è una stringa letterale.
fn string_literal_arg(args: &str) -> Option<String> {
    let first = split_top_level(args).into_iter().next()?;
    string_literal(&first)
}

/// Chiavi di un argomento payload: object literal inline oppure variabile
/// locale il cui inizializzatore è un object literal.
///
/// La seconda forma è quella che il codebase usa quando gli argomenti sono
/// tipizzati (`const args: PresenceToggleArgs = { … }; invoke(cmd, args)`):
/// senza questa risoluzione il test di contratto vedrebbe solo un identificatore
/// e salterebbe la chiamata, cioè esattamente i casi più interessanti.
fn payload_argument_keys(payload: &str, source: &str) -> Option<Vec<String>> {
    let payload = payload.trim();
    if payload.is_empty() {
        return Some(Vec::new());
    }
    if payload.starts_with('{') {
        let (inner, _) = balanced(payload, 0, '{', '}')?;
        return object_keys(inner);
    }
    // Identificatore semplice (niente member access, chiamate, template):
    // si risolve nel file corrente.
    let ident = payload.trim_end_matches(',').trim();
    if ident.is_empty()
        || !ident
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
    {
        return None;
    }
    resolve_local_object_keys(source, ident)
}

/// Cerca `const <ident>` / `let <ident>` (con eventuale annotazione di tipo) il
/// cui inizializzatore sia un object literal, e ne restituisce le chiavi.
fn resolve_local_object_keys(source: &str, ident: &str) -> Option<Vec<String>> {
    for keyword in ["const ", "let "] {
        let needle = format!("{keyword}{ident}");
        for (index, _) in source.match_indices(&needle) {
            let after = index + needle.len();
            // Il nome deve finire qui: `const argsX` non è `const args`.
            if source[after..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
            {
                continue;
            }
            // Salta l'annotazione di tipo (`: SetPresenceDefaultModeArgs`) fino a `=`.
            let Some(offset) = source[after..].find('=') else {
                continue;
            };
            if offset > 240 {
                continue; // troppo lontano: non è l'inizializzatore di questa dichiarazione
            }
            let equals_at = after + offset;
            if source[equals_at..].starts_with("==") {
                continue; // confronto, non assegnamento
            }
            let rest = source[equals_at + 1..].trim_start();
            if !rest.starts_with('{') {
                continue; // ternario/espressione: payload non verificabile staticamente
            }
            let (inner, _) = balanced(rest, 0, '{', '}')?;
            return object_keys(inner);
        }
    }
    None
}

/// Chiavi del secondo argomento di `invoke`. Assente (`{}` implicito) quando
/// `invoke` ha un solo argomento.
fn payload_keys(args: &str, source: &str) -> Option<Vec<String>> {
    let parts = split_top_level(args);
    let Some(payload) = parts.get(1) else {
        return Some(Vec::new());
    };
    payload_argument_keys(payload, source)
}

/// Comandi il cui nome compare nel frontend come stringa letterale, anche
/// quando il nome è scelto a runtime:
///
/// ```ts
/// const command = selectedSource === 'luatools' ? 'trigger_luatools_download' : 'trigger_hubcap_download';
/// await invoke(command, args);
/// ```
///
/// [`frontend_calls`] non può attribuire un payload a queste chiamate (il nome
/// non è letterale nel punto di `invoke`), quindi qui si cerca il nome ovunque:
/// è il criterio giusto per decidere se un comando è "mai usato", mentre le
/// chiavi degli argomenti di questi comandi sono coperte dal test puntuale
/// [`documented_command_contracts_have_the_expected_keys`].
fn referenced_command_names() -> BTreeSet<String> {
    let sources: Vec<String> = frontend_files().iter().map(|path| read(path)).collect();
    registered_commands()
        .into_iter()
        .map(|(_, name)| name)
        .filter(|name| {
            sources
                .iter()
                .any(|source| source.contains(&format!("'{name}'")) || source.contains(&format!("\"{name}\"")))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Ogni comando registrato esiste davvero
// ---------------------------------------------------------------------------

#[test]
fn every_registered_command_exists_in_the_commands_layer() {
    let signatures = command_signatures();
    let mut missing = Vec::new();
    let mut wrong_module = Vec::new();
    for (module, name) in registered_commands() {
        match signatures.get(&name) {
            None => missing.push(format!("commands::{module}::{name}")),
            Some(signature) if signature.module != module => wrong_module.push(format!(
                "commands::{module}::{name} (definito in commands/{}.rs:{})",
                signature.module, signature.line
            )),
            Some(_) => {}
        }
    }
    assert!(
        missing.is_empty(),
        "generate_handler! registra comandi che non esistono: {missing:?}. \
         È la classe di bug del vecchio test inventario (nomi fantasma come \
         `save_installed_lua_manifest_rows`): il frontend riceve \
         \"command not found\" a runtime."
    );
    assert!(
        wrong_module.is_empty(),
        "generate_handler! punta al modulo sbagliato: {wrong_module:?}"
    );
}

#[test]
fn every_command_is_registered_exactly_once() {
    let signatures = command_signatures();
    let registered = registered_commands();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, name) in &registered {
        *counts.entry(name.as_str()).or_default() += 1;
    }
    let duplicates: Vec<_> = counts.iter().filter(|(_, count)| **count > 1).collect();
    assert!(
        duplicates.is_empty(),
        "comandi registrati più volte in generate_handler!: {duplicates:?}"
    );
    let unregistered: Vec<_> = signatures
        .keys()
        .filter(|name| !counts.contains_key(name.as_str()))
        .collect();
    assert!(
        unregistered.is_empty(),
        "comandi con #[tauri::command] ma assenti da generate_handler! (il frontend non può \
         chiamarli): {unregistered:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Ogni invoke del frontend chiama un comando registrato
// ---------------------------------------------------------------------------

#[test]
fn every_frontend_command_name_is_registered() {
    let registered: BTreeSet<String> = registered_commands()
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    let unknown: Vec<String> = frontend_calls()
        .into_iter()
        .filter(|call| !registered.contains(&call.command))
        .map(|call| format!("{}:{} → '{}'", call.file, call.line, call.command))
        .collect();
    assert!(
        unknown.is_empty(),
        "il frontend chiama comandi inesistenti (Promise rigettata a runtime, nessun errore di \
         compilazione): {unknown:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Le chiavi degli argomenti coincidono con le firme Rust
// ---------------------------------------------------------------------------

#[test]
fn frontend_argument_keys_match_the_rust_wire_keys() {
    let signatures = command_signatures();
    let mut unknown_keys = Vec::new();
    let mut missing_keys = Vec::new();

    for call in frontend_calls() {
        let Some(keys) = call.keys.as_ref() else {
            continue; // payload dinamico: coperto dai test puntuali sotto
        };
        let Some(signature) = signatures.get(&call.command) else {
            continue; // già segnalato da every_frontend_command_name_is_registered
        };
        let allowed: BTreeSet<&str> = signature.all_keys().into_iter().collect();
        let sent: BTreeSet<&str> = keys.iter().map(String::as_str).collect();

        for key in sent.difference(&allowed) {
            unknown_keys.push(format!(
                "{}:{} → '{}' manda `{key}` ma la firma Rust accetta {:?} (commands/{}.rs:{})",
                call.file,
                call.line,
                call.command,
                signature.all_keys(),
                signature.module,
                signature.line,
            ));
        }
        for key in signature.required_keys() {
            if !sent.contains(key) {
                missing_keys.push(format!(
                    "{}:{} → '{}' non manda la chiave obbligatoria `{key}` (firma: commands/{}.rs:{})",
                    call.file, call.line, call.command, signature.module, signature.line,
                ));
            }
        }
    }

    assert!(
        unknown_keys.is_empty(),
        "\nChiavi IPC non riconosciute dal backend (Tauri 2 accetta solo la chiave lowerCamelCase \
         derivata dal parametro Rust, senza fallback snake_case):\n  {}\n",
        unknown_keys.join("\n  ")
    );
    assert!(
        missing_keys.is_empty(),
        "\nChiavi obbligatorie mancanti nel payload:\n  {}\n",
        missing_keys.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// 4. Guardie sulla forma delle chiavi wire
// ---------------------------------------------------------------------------

#[test]
fn no_wire_key_is_an_unseparated_compound_word() {
    let suspects: Vec<String> = command_signatures()
        .values()
        .flat_map(|signature| {
            signature.params.iter().filter_map(move |param| {
                let key = param.wire_key.as_str();
                let looks_compound = !key.contains(char::is_uppercase);
                if looks_compound && !COMMON_SINGLE_WORDS.contains(&key) {
                    Some(format!(
                        "`{}` (parametro `{}` di {}, commands/{}.rs:{})",
                        key, param.rust_name, signature.name, signature.module, signature.line
                    ))
                } else {
                    None
                }
            })
        })
        .collect();
    assert!(
        suspects.is_empty(),
        "\nChiavi wire tutte-minuscole fuori dal vocabolario di parole singole note: {suspects:?}\n\
         Se il parametro è davvero una parola singola, aggiungila a COMMON_SINGLE_WORDS in \
         src/tests/ipc_contract_tests.rs. Se è una parola composta (es. `showonline`), rinomina \
         il parametro Rust in snake_case (`show_online` → chiave `showOnline`) e allinea il \
         frontend: Tauri non converte né sana il casing da solo.\n"
    );
}

/// Vocabolario delle chiavi wire di una sola parola accettate dal codice.
const COMMON_SINGLE_WORDS: &[&str] = &[
    "code",
    "content",
    "count",
    "date",
    "edits",
    "enabled",
    "folder",
    "level",
    "name",
    "origin",
    "path",
    "query",
    "request",
    "settings",
    "site",
    "source",
    "start",
    "text",
    "title",
    "url",
];

#[test]
fn no_wire_key_keeps_snake_case() {
    let offenders: Vec<String> = command_signatures()
        .values()
        .flat_map(|signature| {
            signature.params.iter().filter_map(move |param| {
                if param.wire_key.contains('_') {
                    Some(format!(
                        "`{}` in {} (commands/{}.rs:{})",
                        param.wire_key, signature.name, signature.module, signature.line
                    ))
                } else {
                    None
                }
            })
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "chiavi wire con underscore: il frontend dovrebbe mandare camelCase, non snake_case: {offenders:?}"
    );
}

#[test]
fn test_param_names_stay_inside_the_model() {
    // `wire_key()` replica heck solo per parametri snake_case ASCII: questa
    // guardia fallisce se qualcuno introduce maiuscole o cifre iniziali, casi
    // in cui il modello va esteso invece di fidarsi della replica.
    let offenders: Vec<String> = command_signatures()
        .values()
        .flat_map(|signature| {
            signature.params.iter().filter_map(move |param| {
                let name = param.rust_name.trim_start_matches('_');
                let ok = !name.is_empty()
                    && name
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
                    && !name.chars().next().unwrap().is_ascii_digit();
                if ok {
                    None
                } else {
                    Some(format!(
                        "`{}` in {} (commands/{}.rs:{})",
                        param.rust_name, signature.name, signature.module, signature.line
                    ))
                }
            })
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "nomi di parametro fuori dal modello snake_case ASCII: estendere wire_key() prima di \
         procedere, altrimenti il contratto verificato qui non è quello che genera Tauri: {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// 5. Il percorso Steam non è più un parametro del client
// ---------------------------------------------------------------------------

#[test]
fn no_command_takes_a_steam_path_from_the_client() {
    let offenders: Vec<String> = command_signatures()
        .values()
        .flat_map(|signature| {
            signature.params.iter().filter_map(move |param| {
                if param.rust_name.contains("steam_path") {
                    Some(format!(
                        "{} (commands/{}.rs:{}) parametro `{}`",
                        signature.name, signature.module, signature.line, param.rust_name
                    ))
                } else {
                    None
                }
            })
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "\nIl percorso Steam deve essere risolto dal backend con \
         commands::command_steam_path / commands::configured_steam_path, non passato dal client \
         (una lettura impostazioni + decifratura DPAPI per chiamata, e un percorso che può \
         divergere da quello configurato): {offenders:?}\n\
         Eccezione ammessa: check_steam_path, che valida un percorso CANDIDATO prima del save.\n"
    );
}

// ---------------------------------------------------------------------------
// 6. Regressione puntuale sui casi già accaduti
// ---------------------------------------------------------------------------

#[test]
fn documented_command_contracts_have_the_expected_keys() {
    let signatures = command_signatures();
    // (comando, chiavi wire attese nell'ordine della firma)
    let contracts: &[(&str, &[&str])] = &[
        // Il caso #1: parametro Rust `show_online` → chiave `showOnline`.
        // Il VALORE in aethercore.toml resta "showonline" (token di dominio
        // condiviso con AetherDLL): i due livelli non vanno allineati.
        ("set_presence_default_mode", &["showOnline"]),
        // steam_path rimosso: restano solo gli argomenti che il client possiede.
        ("get_installed_lua_manifest_rows", &["appId"]),
        ("get_lua_game_update_state", &["appId"]),
        ("set_lua_game_updates_enabled", &["appId", "enabled"]),
        ("remove_lua_game_from_library", &["appId"]),
        ("apply_specific_version_edits", &["appId", "edits"]),
        ("apply_game_version", &["appId", "buildId"]),
        ("trigger_hubcap_download", &["appId", "apiKey"]),
        ("prepare_specific_version_download", &["appId", "apiKey"]),
        ("trigger_ryuu_download", &["appId", "apiKey"]),
        ("prepare_ryuu_specific_version_download", &["appId", "apiKey"]),
        // `gameName` è opzionale (Option<String>): il client lo passa quando
        // conosce il nome (etichetta la cronologia download lato lua.tools).
        ("trigger_luatools_download", &["appId", "gameName"]),
        ("prepare_luatools_specific_version_download", &["appId", "gameName"]),
        ("install_aether_dll", &["origin"]),
        ("uninstall_aether_dll", &[]),
        ("reset_aether_steam_path", &[]),
        ("probe_aether_steam_residuals", &[]),
        ("get_installed_dll_version", &[]),
        ("check_aether_dll_update", &[]),
        ("is_dll_installed", &[]),
        ("is_steam_blocked", &[]),
        ("block_steam_updates", &[]),
        ("unblock_steam_updates", &[]),
        // Contratti stabili citati in docs/shared_contracts.md.
        ("save_settings", &["settings"]),
        ("get_recent_log_lines", &["tailLines", "source"]),
        ("enable_online", &["appId", "request"]),
        ("plan_online", &["appId"]),
        ("disable_online", &["appId"]),
    ];

    for (command, expected) in contracts {
        let signature = signatures.get(*command).unwrap_or_else(|| {
            panic!("il comando documentato `{command}` non esiste più: aggiornare il contratto")
        });
        assert_eq!(
            signature.all_keys(),
            expected.to_vec(),
            "contratto IPC cambiato per `{command}` (commands/{}.rs:{}): aggiornare il frontend E \
             docs/shared_contracts.md, poi questa lista",
            signature.module,
            signature.line
        );
    }
}

// ---------------------------------------------------------------------------
// 7. Comandi mai chiamati dal frontend
// ---------------------------------------------------------------------------

/// Comandi registrati che il frontend non chiama. Sono ammessi qui solo se
/// inutilizzati **di proposito** (superficie per tooling esterno, flusso non
/// ancora collegato, comando sostituito ma mantenuto per compatibilità).
///
/// Quando un comando nuovo compare in questa lista il test fallisce: è il
/// segnale che o manca il collegamento nella UI o il comando va rimosso.
const DOCUMENTED_UNUSED_COMMANDS: &[(&str, &str)] = &[
    (
        "generate_hubcap_workshop_manifest",
        "la corsia Workshop del sincronizzatore genera i manifest da sola; nessuna UI manuale",
    ),
    (
        "get_custom_css_path",
        "il tema si legge/scrive con read_custom_css + write_custom_css; il percorso non serve alla UI",
    ),
    (
        "get_installed_dll_version",
        "sostituito da check_aether_dll_update, che restituisce già installed_version",
    ),
    (
        "get_personal_wallpaper_path",
        "la UI usa pick_wallpaper_file + usePersonalWallpaper (asset convertito), non il percorso grezzo",
    ),
    (
        "is_uco2_active",
        "sostituito da inspect_foreign_online (vedi il commento in LibraryGameActionsModal.tsx)",
    ),
    (
        "list_lua_history",
        "storico dei backup Lua: nessuna UI collegata",
    ),
    (
        "open_app_folder",
        "apre la cartella di installazione in Explorer: nessun pulsante la invoca",
    ),
    (
        "open_windows_security",
        "apre la pagina Virus & threat protection: nessun pulsante la invoca",
    ),
    (
        "pick_local_folder",
        "la UI usa pick_local_files e pick_steam_folder",
    ),
    (
        "set_session_log_level",
        "il livello di log viaggia dentro save_settings",
    ),
];

#[test]
fn unused_commands_are_only_the_documented_ones() {
    let mut called = referenced_command_names();
    called.extend(frontend_calls().into_iter().map(|call| call.command));
    let unused: BTreeSet<String> = command_signatures()
        .keys()
        .filter(|name| !called.contains(name.as_str()))
        .cloned()
        .collect();
    let documented: BTreeSet<String> = DOCUMENTED_UNUSED_COMMANDS
        .iter()
        .map(|(name, _reason)| name.to_string())
        .collect();

    let unexpected: Vec<&String> = unused.difference(&documented).collect();
    let stale: Vec<&String> = documented.difference(&unused).collect();

    assert!(
        unexpected.is_empty(),
        "\nComandi registrati ma mai chiamati dal frontend: {unexpected:?}\n\
         Collegarli alla UI oppure rimuoverli; se restano inutilizzati di proposito, \
         elencarli in DOCUMENTED_UNUSED_COMMANDS con la motivazione (questa lista è \
         l'inventario vivo della superficie IPC morta: 10 comandi su 119).\n"
    );
    assert!(
        stale.is_empty(),
        "DOCUMENTED_UNUSED_COMMANDS elenca comandi che ora il frontend chiama: rimuoverli dalla \
         lista: {stale:?}"
    );
}
