//! Desk <-> frontend IPC contract, verified against the real sources.
//!
//! # Why this file exists
//!
//! Tauri 2 derives every command argument key with
//! `param.to_lower_camel_case()` and looks it up in the payload with EXACTLY
//! that key (`tauri 2.11.5`, `crates/tauri/src/ipc/command.rs`: `v.get(self.key)`).
//! There is no snake_case fallback. So a wrong frontend key is not a compile
//! error: it is a Promise rejected at runtime with
//! `command X missing required key Y`.
//!
//! The real case that motivated these tests: `set_presence_default_mode`
//! declared `showonline: bool`, an all-lowercase compound word copied from the
//! `default_mode = "showonline"` domain token in `aethercore.toml`. The
//! on-the-wire key was therefore `showonline` and the frontend sent
//! `{ showonline }`: it worked by coincidence. A "cleanup" rename done by only
//! one of the two sides would have broken the presence toggle with neither
//! `tsc` nor `cargo check` saying anything.
//!
//! # What is verified
//!
//! 1. every command registered in `generate_handler!` exists in the command layer;
//! 2. every frontend `invoke('…')` calls a registered command;
//! 3. frontend argument keys match the wire keys derived from the Rust
//!    signatures, and every required key is present;
//! 4. no wire key is a "lowercase compound word" (the bug class above), no key
//!    is an unconverted snake_case;
//! 5. no command receives the Steam path from the client anymore;
//! 6. commands named in the documented contracts have exactly the expected
//!    keys (point regression on cases that already happened);
//! 7. the set of commands never called by the frontend is only the documented one.
//!
//! The tests read the source files instead of using macros/derives because the
//! contract to protect spans two languages: the only point where both halves
//! meet is the text that ends up in the payload.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `AetherDesk/`: the root holding both `src/` (frontend) and `src-tauri/`.
fn desk_dir() -> PathBuf {
    crate_dir()
        .parent()
        .expect("src-tauri/ must live inside AetherDesk/")
        .to_path_buf()
}

fn frontend_dir() -> PathBuf {
    desk_dir().join("src")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// Rust command model
// ---------------------------------------------------------------------------

/// Types injected by the Tauri macro: they never come from the payload, so they
/// have no wire key. Recognized by type (not by name) because the name is free:
/// `enable_online(app_id, request: OnlineEnableRequest)` has a wire param
/// literally called `request`.
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

/// Conventional names used across the codebase for injected handles.
const INJECTED_PARAM_NAMES: &[&str] = &["app", "window", "webview", "state"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct WireParam {
    /// Rust parameter name (`app_id`).
    rust_name: String,
    /// Expected IPC payload key (`appId`).
    wire_key: String,
    /// Declared type, to tell `Option<_>` apart (absent key = None).
    rust_type: String,
}

#[derive(Debug, Clone)]
struct CommandSig {
    /// `library` for `src/commands/library.rs`.
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
/// Replica of `heck::AsLowerCamelCase` for the only alphabet the codebase uses
/// in params (ASCII snake_case): [`test_param_names_stay_inside_the_model`]
/// fails when someone introduces a name with leading capitals or digits, i.e.
/// the only case where this function could diverge from heck.
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

/// Extracts the content between the first `(` (or `[`, `{`) after `from` and its
/// match, honoring nesting. Returns `(inner, index_after)`.
fn balanced(source: &str, from: usize, open: char, close: char) -> Option<(&str, usize)> {
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

/// Splits a param list (Rust) or member list (TS object) on top-level commas:
/// ignores commas inside `()`, `[]`, `{}`, `<>`.
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

/// Signature of every `#[tauri::command]` in the command layer.
fn command_signatures() -> BTreeMap<String, CommandSig> {
    let dir = crate_dir().join("src").join("commands");
    let mut signatures = BTreeMap::new();
    for entry in fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("missing {}: {error}", dir.display()))
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let module = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let source = read(&path);
        for marker in source.match_indices("#[tauri::command") {
            // From the marker to the first public `fn`: other attributes
            // (#[allow], doc comments) may sit in between, but never another
            // signature.
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
                        && !INJECTED_TYPE_MARKERS
                            .iter()
                            .any(|marker| ty.contains(marker))
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
            let previous_line = previous
                .as_ref()
                .map(|signature| signature.line)
                .unwrap_or(0);
            assert!(
                previous.is_none(),
                "duplicate command `{name}`: commands/{module}.rs:{line} and                  commands/{previous_module}.rs:{previous_line}. Two #[tauri::command] with the                  same name make the generate_handler! registration ambiguous!"
            );
        }
    }
    signatures
}

/// Commands registered in `generate_handler![…]` (`src/main.rs`), as
/// `(module, name)`.
fn registered_commands() -> Vec<(String, String)> {
    let source = read(&crate_dir().join("src").join("main.rs"));
    let start = source
        .find("generate_handler!")
        .expect("src/main.rs must register commands with generate_handler!");
    let (body, _) = balanced(&source, start, '[', ']').expect("unbalanced generate_handler![…]");
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
                "unexpected generate_handler! entry: `{token}` (expected commands::<module>::<name>)"
            ),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Frontend call model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FrontendCall {
    file: String,
    line: usize,
    command: String,
    /// Payload object keys; `None` = non-literal payload (variable, spread,
    /// dynamic construction): the call is not verifiable here.
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

/// Top-level keys of a TS object literal.
///
/// Covers the shapes used across the codebase: `key: value`, shorthand `key`,
/// `'key': value`, computed key/spread (which make the payload unverifiable
/// → `None`).
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

/// Every frontend call into the backend: direct `invoke('…')` and
/// `queryGameState(appId, '…')` (which adds `appId` to the payload itself).
fn frontend_calls() -> Vec<FrontendCall> {
    let mut calls = Vec::new();
    for path in frontend_files() {
        let source = read(&path);
        let relative = path
            .strip_prefix(desk_dir())
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        // --- invoke<T>('name', payload) / invoke('name', payload) -----------
        for (index, _) in source.match_indices("invoke") {
            if index > 0 {
                let previous = source[..index].chars().next_back().unwrap();
                if previous.is_ascii_alphanumeric() || previous == '_' || previous == '.' {
                    continue; // `reinvoke`, `foo.invoke`: not the IPC function
                }
            }
            let after = index + "invoke".len();
            // `invoke<…>(` or `invoke(`: skip the optional type argument.
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
            // The provider always adds `appId`; the third argument (when present)
            // is the object with the remaining keys.
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
        .or_else(|| {
            raw.strip_prefix('"')
                .and_then(|inner| inner.strip_suffix('"'))
        })?;
    Some(inner.to_string())
}

/// First argument, when it is a string literal.
fn string_literal_arg(args: &str) -> Option<String> {
    let first = split_top_level(args).into_iter().next()?;
    string_literal(&first)
}

/// Keys of a payload argument: an inline object literal, or a local variable
/// whose initializer is an object literal.
///
/// The second shape is what the codebase uses when arguments are typed
/// (`const args: PresenceToggleArgs = { … }; invoke(cmd, args)`): without this
/// resolution the contract test would only see an identifier and skip the
/// call — i.e. exactly the most interesting cases.
fn payload_argument_keys(payload: &str, source: &str) -> Option<Vec<String>> {
    let payload = payload.trim();
    if payload.is_empty() {
        return Some(Vec::new());
    }
    if payload.starts_with('{') {
        let (inner, _) = balanced(payload, 0, '{', '}')?;
        return object_keys(inner);
    }
    // Plain identifier (no member access, calls, templates): resolved in the
    // current file.
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

/// Finds `const <ident>` / `let <ident>` (with optional type annotation) whose
/// initializer is an object literal, and returns its keys.
fn resolve_local_object_keys(source: &str, ident: &str) -> Option<Vec<String>> {
    for keyword in ["const ", "let "] {
        let needle = format!("{keyword}{ident}");
        for (index, _) in source.match_indices(&needle) {
            let after = index + needle.len();
            // The name must end here: `const argsX` is not `const args`.
            if source[after..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
            {
                continue;
            }
            // Skip the type annotation (`: SetPresenceDefaultModeArgs`) up to `=`.
            let Some(offset) = source[after..].find('=') else {
                continue;
            };
            if offset > 240 {
                continue; // too far: not this declaration's initializer
            }
            let equals_at = after + offset;
            if source[equals_at..].starts_with("==") {
                continue; // comparison, not assignment
            }
            let rest = source[equals_at + 1..].trim_start();
            if !rest.starts_with('{') {
                continue; // ternary/expression: payload not statically verifiable
            }
            let (inner, _) = balanced(rest, 0, '{', '}')?;
            return object_keys(inner);
        }
    }
    None
}

/// Keys of `invoke`'s second argument. Absent (implicit `{}`) when `invoke`
/// takes a single argument.
fn payload_keys(args: &str, source: &str) -> Option<Vec<String>> {
    let parts = split_top_level(args);
    let Some(payload) = parts.get(1) else {
        return Some(Vec::new());
    };
    payload_argument_keys(payload, source)
}

/// Commands whose name appears in the frontend as a string literal, even when
/// the name is picked at runtime:
///
/// ```ts
/// const command = selectedSource === 'luatools' ? 'trigger_luatools_download' : 'trigger_hubcap_download';
/// await invoke(command, args);
/// ```
///
/// [`frontend_calls`] cannot attribute a payload to these calls (the name is
/// not literal at the `invoke` site), so here the name is searched for
/// everywhere: that is the right criterion to decide whether a command is
/// "never used", while these commands' argument keys are covered by the point
/// test [`documented_command_contracts_have_the_expected_keys`].
fn referenced_command_names() -> BTreeSet<String> {
    let sources: Vec<String> = frontend_files().iter().map(|path| read(path)).collect();
    registered_commands()
        .into_iter()
        .map(|(_, name)| name)
        .filter(|name| {
            sources.iter().any(|source| {
                source.contains(&format!("'{name}'")) || source.contains(&format!("\"{name}\""))
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Every registered command really exists
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
                "commands::{module}::{name} (defined in commands/{}.rs:{})",
                signature.module, signature.line
            )),
            Some(_) => {}
        }
    }
    assert!(
        missing.is_empty(),
        "generate_handler! registers commands that do not exist: {missing:?}. \
         This is the bug class of the old inventory test (ghost names like \
         `save_installed_lua_manifest_rows`): the frontend gets \
         \"command not found\" at runtime."
    );
    assert!(
        wrong_module.is_empty(),
        "generate_handler! points at the wrong module: {wrong_module:?}"
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
        "commands registered more than once in generate_handler!: {duplicates:?}"
    );
    let unregistered: Vec<_> = signatures
        .keys()
        .filter(|name| !counts.contains_key(name.as_str()))
        .collect();
    assert!(
        unregistered.is_empty(),
        "commands with #[tauri::command] but missing from generate_handler! (the frontend cannot \
         call them): {unregistered:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Every frontend invoke calls a registered command
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
        "the frontend calls commands that do not exist (Promise rejected at runtime, no \
         compile error): {unknown:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Argument keys match the Rust signatures
// ---------------------------------------------------------------------------

#[test]
fn frontend_argument_keys_match_the_rust_wire_keys() {
    let signatures = command_signatures();
    let mut unknown_keys = Vec::new();
    let mut missing_keys = Vec::new();

    for call in frontend_calls() {
        let Some(keys) = call.keys.as_ref() else {
            continue; // dynamic payload: covered by the point tests below
        };
        let Some(signature) = signatures.get(&call.command) else {
            continue; // already reported by every_frontend_command_name_is_registered
        };
        let allowed: BTreeSet<&str> = signature.all_keys().into_iter().collect();
        let sent: BTreeSet<&str> = keys.iter().map(String::as_str).collect();

        for key in sent.difference(&allowed) {
            unknown_keys.push(format!(
                "{}:{} → '{}' sends `{key}` but the Rust signature accepts {:?} (commands/{}.rs:{})",
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
                    "{}:{} → '{}' does not send the required key `{key}` (signature: commands/{}.rs:{})",
                    call.file, call.line, call.command, signature.module, signature.line,
                ));
            }
        }
    }

    assert!(
        unknown_keys.is_empty(),
        "\nIPC keys unrecognized by the backend (Tauri 2 only accepts the lowerCamelCase \
         key derived from the Rust parameter, with no snake_case fallback):\n  {}\n",
        unknown_keys.join("\n  ")
    );
    assert!(
        missing_keys.is_empty(),
        "\nRequired keys missing from the payload:\n  {}\n",
        missing_keys.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// 4. Guards on the wire key shape
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
                        "`{}` (param `{}` of {}, commands/{}.rs:{})",
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
        "\nAll-lowercase wire keys outside the known single-word vocabulary: {suspects:?}\n\
         If the parameter really is a single word, add it to COMMON_SINGLE_WORDS in \
         src/tests/ipc_contract_tests.rs. If it is a compound word (e.g. `showonline`), rename \
         the Rust parameter to snake_case (`show_online` → key `showOnline`) and align the \
         frontend: Tauri converts and heals no casing by itself.\n"
    );
}

/// Vocabulary of single-word wire keys accepted by the code.
const COMMON_SINGLE_WORDS: &[&str] = &[
    "base", // expected settings snapshot for conflict-aware three-way patch
    "code", "content", "count", "date", "edits", "enabled", "folder", "level", "name", "origin",
    "path", "query", "request", "settings", "site", "source", "start", "text", "title", "url",
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
        "wire keys with underscores: the frontend should send camelCase, not snake_case: {offenders:?}"
    );
}

#[test]
fn test_param_names_stay_inside_the_model() {
    // `wire_key()` replicates heck only for ASCII snake_case params: this guard
    // fails when someone introduces leading capitals or digits, cases where the
    // model must be extended instead of trusting the replica.
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
        "parameter names outside the ASCII snake_case model: extend wire_key() before \
         proceeding, otherwise the contract verified here is not the one Tauri generates: {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// 5. The Steam path is no longer a client parameter
// ---------------------------------------------------------------------------

#[test]
fn no_command_takes_a_steam_path_from_the_client() {
    let offenders: Vec<String> = command_signatures()
        .values()
        .flat_map(|signature| {
            signature.params.iter().filter_map(move |param| {
                if param.rust_name.contains("steam_path") {
                    Some(format!(
                        "{} (commands/{}.rs:{}) param `{}`",
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
        "\nThe Steam path must be resolved by the backend with \
         commands::command_steam_path / commands::configured_steam_path, not passed by the client \
         (one settings read + DPAPI decryption per call, and a path that may \
         diverge from the configured one): {offenders:?}\n\
         Allowed exception: check_steam_path, which validates a CANDIDATE path before save.\n"
    );
}

// ---------------------------------------------------------------------------
// 6. Point regression on cases that already happened
// ---------------------------------------------------------------------------

#[test]
fn documented_command_contracts_have_the_expected_keys() {
    let signatures = command_signatures();
    // (command, expected wire keys in signature order)
    let contracts: &[(&str, &[&str])] = &[
        // Case #1: Rust param `show_online` → key `showOnline`.
        // The VALUE in aethercore.toml stays "showonline" (domain token shared
        // with AetherDLL): the two levels must not be aligned.
        ("set_presence_default_mode", &["showOnline"]),
        // steam_path removed: only args the client owns remain.
        ("get_installed_lua_manifest_rows", &["appId"]),
        ("get_lua_game_update_state", &["appId"]),
        ("set_lua_game_updates_enabled", &["appId", "enabled"]),
        ("remove_lua_game_from_library", &["appId"]),
        ("apply_specific_version_edits", &["appId", "edits"]),
        ("apply_game_version", &["appId", "buildId"]),
        ("trigger_hubcap_download", &["appId", "apiKey"]),
        ("prepare_specific_version_download", &["appId", "apiKey"]),
        ("trigger_ryuu_download", &["appId", "apiKey"]),
        (
            "prepare_ryuu_specific_version_download",
            &["appId", "apiKey"],
        ),
        // `gameName` is optional (Option<String>): the client sends it when it
        // knows the name (labels the download history on the lua.tools side).
        ("trigger_luatools_download", &["appId", "gameName"]),
        (
            "prepare_luatools_specific_version_download",
            &["appId", "gameName"],
        ),
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
        // Stable contracts quoted in docs/shared_contracts.md.
        ("save_settings", &["settings", "base"]),
        ("get_recent_log_lines", &["tailLines", "source"]),
        ("enable_online", &["appId", "request"]),
        ("plan_online", &["appId"]),
        ("disable_online", &["appId"]),
    ];

    for (command, expected) in contracts {
        let signature = signatures.get(*command).unwrap_or_else(|| {
            panic!("documented command `{command}` no longer exists: update the contract")
        });
        assert_eq!(
            signature.all_keys(),
            expected.to_vec(),
            "IPC contract changed for `{command}` (commands/{}.rs:{}): update the frontend AND \
             docs/shared_contracts.md, then this list",
            signature.module,
            signature.line
        );
    }
}

// ---------------------------------------------------------------------------
// 7. Commands never called by the frontend
// ---------------------------------------------------------------------------

/// Registered commands the frontend never calls. Allowed here only when
/// unused **on purpose** (surface for external tooling, not-yet-wired flow,
/// replaced command kept for compatibility).
///
/// When a new command shows up in this list the test fails: it signals either
/// a missing UI wiring or a command to remove.
const DOCUMENTED_UNUSED_COMMANDS: &[(&str, &str)] = &[
    (
        "generate_hubcap_workshop_manifest",
        "the synchronizer's Workshop lane generates manifests on its own; no manual UI",
    ),
    (
        "get_custom_css_path",
        "the theme is read/written with read_custom_css + write_custom_css; the UI never needs the path",
    ),
    (
        "get_hubcap_monitor_status",
        "the sync-status popup was removed and the monitor runs headless; the command stays as the diagnostic surface",
    ),
    (
        "get_installed_dll_version",
        "replaced by check_aether_dll_update, which already returns installed_version",
    ),
    (
        "get_personal_wallpaper_path",
        "the UI uses pick_wallpaper_file + usePersonalWallpaper (converted asset), not the raw path",
    ),
    (
        "is_uco2_active",
        "replaced by inspect_foreign_online (see the comment in LibraryGameActionsModal.tsx)",
    ),
    (
        "list_lua_history",
        "Lua backup history: no UI wired to it",
    ),
    (
        "open_app_folder",
        "opens the install folder in Explorer: no button invokes it",
    ),
    (
        "open_windows_security",
        "opens the Virus & threat protection page: no button invokes it",
    ),
    (
        "pick_local_folder",
        "the UI uses pick_local_files and pick_steam_folder",
    ),
    (
        "set_session_log_level",
        "the log level travels inside save_settings",
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
        "\nCommands registered but never called by the frontend: {unexpected:?}\n\
         Wire them to the UI or remove them; when they stay unused on purpose, \
          list them in DOCUMENTED_UNUSED_COMMANDS with the reason (this list is \
         the live inventory of the dead IPC surface: 11 commands out of 119).\n"
    );
    assert!(
        stale.is_empty(),
        "DOCUMENTED_UNUSED_COMMANDS lists commands the frontend now calls: remove them from \
         the list: {stale:?}"
    );
}
