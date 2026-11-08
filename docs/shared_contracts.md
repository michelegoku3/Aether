# Shared contracts — AetherDesk ↔ AetherDLL

Single specification of the **twinned** logic between AetherDesk (Rust/Tauri)
and AetherDLL (C++, in-process in Steam). The two sides run in different
processes, at different times (Desk may be closed; the DLL lives inside Steam)
yet must produce **the same backup tree, the same cache and the same quota
consumption**. This page is the source of truth for the *policy*; the code
implements it twice, the contract tests (§6) protect it.

> **Maintenance rule**: when you change a value or a rule here, you must align
> BOTH implementations and update this table in the same commit. Every section
> lists the two "twins" with precise paths.

---

## 1. On-disk layout (single source of truth)

| Path | Contents | Writers |
|---|---|---|
| `<Steam>\depotcache\<depot>_<gid>.manifest` | Steam's primary cache | Steam, Desk (resolver/workshop/store), DLL (live generation, restore) |
| `<Steam>\config\depotcache\` | Steam's secondary cache (read-only for us) | Steam |
| `<AetherData>\backup\<app_id>\lua\<app_id>.lua` | **original** Lua (pristine, from the sources) | Desk (`store_original`) |
| `<AetherData>\backup\<app_id>\lua\history\<app>-<millis>[-n].lua` | modified versions, content-deduplicated | Desk (`archive_lua_history`), DLL ("as-missing" copy only) |
| `<AetherData>\backup\<app_id>\lua\<depot>_<gid>.manifest` | referenced manifests (active AND commented pins) + Workshop manifests | Desk and DLL (contract §2) |
| `<AetherData>\state\hubcap_generation_quota.json` (+ `.lock` sibling) | shared daily budget | Desk and DLL (contract §4) |
| `<AetherData>\temp\hubcap\workshop\<item>.manifest` | per-item keyed Workshop cache (Desk only) | Desk |
| `<AetherData>\state\hubcap_game_updates.json` | monitor checkpoint (fingerprint, `contents_checked`, …) | Desk only |

The `<depot>_<gid>.manifest` file name is the manifest's **complete identity**:
two different bytes under the same name do not happen in practice; hence the
copy contract never overwrites (§2).

## 2. Manifest backup/restore contract

Twins:
- Desk: `AetherDesk/src-tauri/src/core/backup.rs` → `backup_referenced_manifests`,
  `backup_manifest_bytes`, `sync_lua_backups_from_stplug_in`.
- DLL: `AetherDLL/AetherCore/hooks/wire/ManifestRestore.cpp` →
  `CopyAtomicallyIfMissing`, `BackupReferencedManifestsAtStartup`,
  `BackupManifestAfterGeneration`, `RestoreLuaDirectory`/`RestoreAppOnce`.

Rules (identical in both processes):
1. only manifests referenced by the current Lua are copied (active or commented
   pins — the regex is not line-anchored, so `--setManifestid` counts as a
   reference); Workshop manifests have a dedicated channel
   (`backup_manifest_bytes` on the Desk side, keyed on the owning app);
2. a **non-empty destination is never overwritten**;
3. a **0-byte destination is a placeholder**: remove and rewrite it;
4. the copy is **atomic** (temp + rename/`AtomicReplace`) and **size-verified**;
5. a failed copy **rechecks the destination**: when the other process completed
   inside the race window, the file counts as present, not as an error.

Restore order (local-first, identical in both):
`depotcache` → `config\depotcache` → `backup\<app_id>\lua`.
When the DLL finds the file in the backup, it **publishes** it to depotcache
before answering hit; Desk (`manifest/resolver.rs`) does the same
restore-then-gate. An empty file is NEVER a local hit.

**Lua** backup: the original is the pristine version (first Lua seen or last
downloaded from the sources); every differing content goes to `history/`,
byte-deduplicated. Archives within the same millisecond take an `-n` suffix
(pin pipelines archive pre+post back-to-back). Anyone rewriting a Lua must
archive **pre AND post** (see §5, matrix in report §12).

## 3. "Manifest first, Lua last" commit contract

The Lua in `config\stplug-in\` is the DLL hot-reload **trigger**: the moment it
appears, every manifest it references must already be visible in depotcache.
Implementations:
- Desk: `steam/compat.rs` → `install_lua_and_manifest_files` (staging → remove
  old targets → rename manifests → verify → rename Lua → verify).
- Desk: `manifest/pin_refresh.rs` → `refresh_game_pins_from_hubcap` (completeness
  gate BEFORE `realign_commented_pins`).
- DLL: live generation writes to depotcache (temp + `AtomicReplace` + size
  verification) without ever touching the Lua.
- Desk: `application/store/package_completion.rs` → `complete_luatools_package`.
  LuaTools sources that archive manifests answer `/api/manifest/download` with
  `<appid>.zip` (Lua + `.manifest`); sources replicating only the Lua (e.g. Luie)
  answer with a naked Lua whose pins are not on disk. Before
  `install_lua_and_manifest_files`, enabled pins missing from the package and
  not already local are fetched from `GET /api/givemethemanifestpunk/{depot}/{gid}`
  (same session, depot+GID identity check via `manifest::identity`, no daily cap
  consumed); only the remainder goes to Hubcap generation, and only when the
  user has a configured key. Sources are tried in "brings manifests" →
  "Lua only" order (`rank_available_sources`); when the first package cannot be
  completed, ONE fallback source is tried (`LUATOOLS_MAX_SOURCE_ATTEMPTS`): every
  attempt costs one cap download.

## 4. Hubcap quota and generation contract

Twins: `AetherDesk/src-tauri/src/providers/hubcap_generation.rs` ↔
`AetherDLL/AetherCore/network/HubcapQuota.cpp`.

- Daily budget: **1500 game/day**, **500 workshop/day**; reset at fixed **EST
  midnight** (like the provider), not local midnight.
- One state file + `.lock` sibling: every read-modify-write cycle happens under
  lock, so counters stay coherent no matter which process writes.
- **reserve → request → release-on-failure**: a failed attempt returns the
  reserved unit (no transient error burns quota).
- The DLL reserves game generations only (workshop is Desk-only).
- Game generation endpoint: `GET /generate/manifest?depot_id=&manifest_id=`;
  the response must be identity-validated (depot+gid in the protobuf) before
  install.

Traffic policy (current values — align here when they change):

| Parameter | Desk | DLL |
|---|---|---|
| Generation serialization | process scheduler, min interval **750 ms** | serialized worker (queue), no long in-pass retries |
| Attempts per generation | 1 (retry lives at lane level) | **2** × **6 s** timeout, 750 ms sleep between attempts |
| Backoff on failure | monitor lane: **30 s × 2^n**, cap **30 min** | proactive: **{30, 60, 120, 300, 600, 900} s**, max **6** retries |
| Bridge window (CM recv) | — | `manifestBridgeWaitMs` default **2500 ms** (clamp 0–10000), then passthrough + background continuation |
| Cap per pass | workshop: **5** remote generations/run (deferred → re-run after **90 s**); pin_refresh: 1 app/poll, **15 min** interval | proactive queue: 1 (gid,depot) key at a time, dedup per key |
| Poll/cadence | monitor: start delay **8 s**, poll **20 s** | DirWatch/event-driven + proactive queue |

Asymmetries are **intentional**: the DLL sits inside Steam's network thread (must
answer in ~ms and cannot queue infinite work), Desk is a UI orchestrator (can
afford periodic lanes and batches). The values must stay compatible though:
e.g. the bridge window (2.5 s) ≥ live-generation p95 (~1 s); the DLL backoff
(max 15 min) ≈ Desk backoff cap (30 min).

### Saved-key health

Twins: `AetherDesk/src-tauri/src/providers/hubcap/key_status.rs` ↔
`AetherDLL/AetherCore/security/HubcapKeyCheck.h`.

The provider exposes no expiry date, so the key's state is classified from one
`/user/stats` answer:

- 401/403, or a `2xx` answer with `can_make_requests=false` → **inactive**.
- `2xx` with the flag `true` or absent → **active**.
- Transport failure, 408/425, 429, `5xx` or any other status → **unknown**: it
  neither raises the warning nor clears a previous one, because an outage is not
  evidence about the key.

- Desk checks the saved key once at startup (`get_hubcap_usage` performs the
  full round-trip) and shows one warning per session; every later Hubcap 401/403
  publishes the same verdict through `hubcap://key-status`, so the warning also
  appears when the key dies while the app is open. The event carries a short
  fingerprint of its key, so a request still in flight for a replaced key cannot
  flag the new one.
- Saving a key that the validation already reported as not active refreshes the
  badge with `quiet: true`: no second modal on top of the inline message.
- The DLL probes `/user/stats` once per Steam session on its own worker, writes
  `hubcap_key_state` and `hubcap_key_checked_epoch` into
  `<Steam>\aethercore\status.json`, and shows one system warning box when the
  key is inactive.
- No saved key means nothing to check and nothing to show on either side.
- A rejected key is **kept** so the user can replace it; the Settings save never
  deletes it on a validation failure.

## 5. Lua pin contract

- **Active** pin (`setManifestid(depot,"gid")`) = version lock: Steam must stay
  on that gid; automatic pipelines NEVER touch it.
- **Commented** pin = informational: the game is in update mode; the commented
  value must point at the last known version because "Disable updates" turns
  it back on.
- **GIDs are not chronologically ordered**: "newer" does not exist; the signal
  is "different from the pin", prioritizing local evidence
  (`latest_installed_gid`, mtime on depotcache) over the Hubcap view
  (`/manifest/{appid}/contents`, free endpoint).
- Every Lua rewrite (realign, toggle, policy, version, edits) must archive
  pre+post to `history/` and back up the referenced manifests (full matrix:
  report §12).

## 6. Contract tests

- **Desk** (`cargo test` in `AetherDesk/src-tauri`):
  `tests/ipc_contract_tests.rs` — §7/§8 rules: registered vs defined commands,
  frontend-invoked names vs registered, argument keys vs Rust signatures, wire
  key shape, no `steam_path` in payloads, unused-command inventory.
  `core/backup.rs::tests` — copy-contract rules 2/3/4 (`backup_manifest_bytes`),
  0-byte placeholder replacement, rejected empty payload, `store_original`
  (Created/Updated/Unchanged), `store_history_version` dedup, same-millisecond
  archive uniqueness.
- **DLL**: `ctest` via `build_dll.cmd`; the `core_unit` case `hubcap_key` pins
  the saved-key classification (`security/HubcapKeyCheck.h`). Manual checklist
  when touching `ManifestRestore.cpp`/`ManifestFetch.cpp`:
  1. copy with non-empty destination → unchanged; 2. 0-byte destination →
  replaced; 3. post-copy size verified; 4. Desk/DLL race → no spurious error
  (rule 5); 5. `BackupManifestAfterGeneration` also catches commented pins;
  6. quota: release on failure.
- When a §2–§5 rule changes: update both twins, this page and the tests in the
  same commit.

---

## 7. Desk → frontend IPC contract (argument keys)

The frontend calls commands with `invoke('command_name', { key: value })`.

**Rule**: Tauri 2 derives every argument's key from the Rust parameter name with
`to_lower_camel_case()` and looks it up in the payload with **exactly** that
key (`tauri 2.11.5`, `crates/tauri/src/ipc/command.rs`: `v.get(self.key)`).
There is no snake_case fallback: a wrong key is not a compile error, it is a
Promise rejected at runtime with `command X missing required key Y`.

| Rust parameter | Payload key | Notes |
|---|---|---|
| `app_id: u32` | `appId` | general rule |
| `tail_lines: Option<usize>` | `tailLines` | `Option` → omittable key |
| `game_name: Option<String>` | `gameName` | `trigger_luatools_download` / `prepare_luatools_specific_version_download`: optional hint forwarded to lua.tools (`&game_name=`), labels only the account's download history |
| `show_online: bool` | `showOnline` | §7.1 |
| `allow_steam_kill: bool` | `allowSteamKill` | `install_aether_dll` / `uninstall_aether_dll` / `reset_aether_steam_path`: `true` only after the user accepts the "Close Steam?" popup; the backend then terminates Steam (if still running) and every running game from its libraries before writing into the Steam folder |

### 7.1 `showonline` is a value, not a key

`aethercore.toml` holds `[presence] default_mode = "showonline"` and the
`showonline_apps` / `aetheronline_apps` / `exclude_apps` arrays. That
**all-lowercase** token is also read by AetherDLL (`AetherCore/core/Settings.cpp`):
shared domain vocabulary, not casing to fix.

The command setting it, though, crosses IPC:

- Rust parameter: `show_online: bool` → wire key `showOnline`
  (`commands/steam.rs::set_presence_default_mode`);
- frontend payload: `{ showOnline: next }`
  (`src/views/settings/SettingsAetherSection.tsx`);
- value written to TOML: the `"showonline"` string
  (`core/presence_config.rs::set_default_mode_in_toml`).

Historically the parameter was named `showonline` (domain token copied into the
parameter name) and the frontend sent `{ showonline }`: it worked by coincidence,
and a "cleanup" rename from only one of the two sides would have silently broken
the presence toggle. Keep the three levels above distinct.

### 7.2 Adding or renaming a command

1. signature in `src-tauri/src/commands/<module>.rs` with `#[tauri::command]`;
2. registration in `generate_handler!` (`src-tauri/src/main.rs`);
3. frontend payload with the lowerCamelCase keys derived from the signature;
4. `cargo test ipc_contract` — `tests/ipc_contract_tests.rs` verifies 1↔2↔3 on
   the real sources and is updated only when the contract changes **on purpose**
   (the `documented_command_contracts_have_the_expected_keys` and
   `DOCUMENTED_UNUSED_COMMANDS` lists).

Style rule: with more than one argument, declare them in a type alias in
`src/types/` (e.g. `PresenceToggleArgs`) instead of inline. **`type`, not
`interface`**: `invoke()` takes `Record<string, unknown>` and an `interface`
has no implicit index signature, so it is not assignable (TS2345).

---

## 8. The Steam path is not a client parameter

**Rule**: no command receives `steam_path` from the frontend. The path resolves
in the backend, from settings, with two helpers in
`src-tauri/src/commands/mod.rs`:

| Helper | When | Behavior without a path |
|---|---|---|
| `command_steam_path(&app)` | the command cannot work without a valid path (install/uninstall, manifest writes, downloads) | `Err` with a user message ("Steam installation path is not configured…") + filesystem validation |
| `configured_steam_path(&app)` | the command must degrade gracefully (DLL version reads, update checks, residue probes) | `None` → the caller answers `N/A` / `0` / "no updates" |

**Allowed exception**: `commands/steam.rs::check_steam_path(path)` validates a
**candidate** path (what the user is typing in Settings, before save). It is
not the configured path, so the client must pass it.

**Why**: with the parameter in the payload every call cost the UI a
`get_settings` (file read + DPAPI keystore decryption) and 22 signatures where
the client could send a path different from the configured one. The rule is
guarded by `tests/ipc_contract_tests.rs::no_command_takes_a_steam_path_from_the_client`.

## 9. Desk mutation ownership (settings batch/coordinator, 2026-10-05)

### Settings and DLL configuration

- `SettingsManager::update` runs **strict load → typed patch → normalization → commit** under mutex and advisory `settings.lock`. The closure must be pure: no remote I/O, no recursive repository calls.
- IPC `save_settings` takes `settings` **and `base`** (the form-seen snapshot) and returns the effective snapshot. Fields unchanged vs `base` keep the current value. A same-field conflict returns `SETTINGS_CONFLICT`, with no sensitive values in the message.
- `settings.json` and `provider_credentials.dat` keep their existing formats. `settings.pending.json` holds a journal version, sanitized JSON and DPAPI ciphertext: published before both files and replayed idempotently before every repository read/mutation. `settings.last-good.json` keeps an older pair in the **same journal format**, not the plain settings JSON format.
- Do not confuse a present journal with an abandoned lock: the journal is recovered, while `.lock` files may legitimately linger after stop; lock ownership belongs to the OS.
- `load()` stays a read-only adapter with logged fallback; write commands and `get_settings` use strict loading. A parse/DPAPI error never authorizes saving the defaults.
- Every Desk editor of `aethercore.toml` goes through `core/config_document.rs`: lossless parser, section-aware keys, per-document lock and unique temps. The legacy copy is updated only when present. The two TOML copies are not a multi-file transaction: a sync error is reported, not declared success.
- Rust 1.89 minimum for `std::fs::File` advisory locks; no new locking library. `toml_edit` becomes a direct dependency, reusing the version already in the lockfile.

### Per-game mutations

- `core/game_mutations.rs` identifies the resource with **canonicalized Steam root + AppID**. Equivalent aliases converge; on Windows the key is case-insensitive. The map keeps weak references to avoid growing forever.
- Remote operations: `MutationPlan::prepare` BEFORE reading inputs/preparing the download; `commit_for` immediately before publication. Meanwhile no game lock is held for networking. The commit checks Desk generation, Lua/ACF/libraryfolders fingerprint and current root. A concurrent change returns `GAME_STATE_CONFLICT` without publishing the Lua/ACF prepared on the old state. Already-restored immutable manifests from the resolver may stay cached.
- Synchronous operations: `try_acquire` returns `GAME_BUSY` instead of blocking the runtime. Async lanes may await the same game's mutex. Coordination is in-memory only and creates no `desk-locks` files. Distinct Desk processes are not coordinated by this mechanism.
- **Lock order:** game → depotcache/config-editor gate. Never re-acquire the same game from a helper: the post-install Store refresh starts AFTER the main transaction releases. The shared realignment receives the guard from the caller.
- When delegating to `spawn_blocking`, the worker receives the guard by value: cancelling the waiter must not free a lock while the worker is still writing. This also holds for the external-tool and online-deploy wrappers.
- Local imports keep ownership during writes, policy update and final backup. Bulk acquires discovered AppIDs in order before touching files. Manifest Workshop writers, which never rewrite the game's Lua/ACF, keep their own gate/cache identity contract.
- **Deliberate limit:** Steam, the DLL, manual editors and old Desk builds do not obey the new locks. Fingerprint and pre-replacement recheck detect observable changes but offer no atomic CAS against uncooperative external writers. Do not block the Steam process to fake this guarantee.
- The coordinator is not a multi-file rollback journal and does not solve the read-modify-write of global JSON cache/online state across different AppIDs: those are distinct domains.

Diagnostics logs: `settings` (prepared/applying/finalized/conflict), `config` (operation/path/changed/error, never values), `mutations` (begin/release with ID, operation, AppID/root, duration; busy/conflict). **Release means freed resource, not successful operation**: the outcome stays in the caller's log.
## 10. Log line contract

`desk.log` and `main.log` are read as one document: the Logs view merges them
with each other and with `uc_online2.log`, and `export_logs_bundle` ships them
together as a support artifact. One shape, both halves:

```
[YYYY-MM-DD HH:MM:SS.mmm] [PID:12345] [TID:1234] [INFO ] [Module    ] message
```

- **Absolute local date, always.** It is the merge key: `commands/logs.rs`
  orders merged lines on the first bracket (`sort_key`, which also feeds the
  line's position in the export), so a bare `HH:MM:SS` cannot be ordered across
  midnight and cannot be placed next to Windows' own logs. Fixed width:
  millisecond precision, zero padded.
- **`PID` is Desk-only.** `main.log` is written by Steam and by every game
  process under one cross-process mutex, so a PID there would add bytes without
  identifying anything the `TID` does not.
- **Level tags are fixed width** (`TRACE`, `DEBUG`, `INFO `, `WARN `, `ERROR`),
  because the view derives the line class from the bracketed tag.
- **Module tag** names the concern (`store`, `library-watch`, `Depot`,
  `License.Hooks`); Desk capitalizes the first letter, the DLL does not. The
  merged view prepends its own source tag instead: `[DESK]`, `[DLL ]`, `[UCO2]`.
- **No ANSI, no multi-line messages.** A stack or a payload is one line.

Twins:
- Desk: `core/logger.rs` → `format_line` (shape) and `format_timestamp_ms`
  (date); merged ordering and source tags in `commands/logs.rs` →
  `merge_tagged`, `sort_key`.
- DLL: `core/Logger.cpp` → `FormatTimeAndThread` (date, buffer 64) and `Emit` /
  `EmitSync` (shape).

Decision under test: `core/logger.rs::tests::formatted_line_carries_an_absolute_date_and_the_shared_shape`
and the `wire_logger_line_carries_the_date` ctest case assert the same shape
from both halves; `commands/logs.rs::tests::merged_logs_sort_across_days_and_sources`
covers the ordering the date exists for.

### 10.1 Export bundle layout (Desk only)

The `Save` button in the Logs view ships
`Downloads\AetherLogs_HH-MM-SS.zip` (built by
`AetherDesk/src-tauri/src/commands/logs.rs::export_logs_bundle_sync`) with
one folder per artifact family:

```
AetherLog/  desk.log.txt, desk.log.last.txt, desk_status.json.txt
            aetherdll_main.log.txt, aetherdll_main.log.last.txt, aetherdll_status.json.txt
            uc_online2.log.txt (normalized to [UCO2]), donors.txt (no time suffix)
SteamLog/   <original Steam names, already filtered>
```

- `AetherLog/` keeps a time suffix (`desk.log_14-32-05.txt`) because each
  export is its own snapshot; files carry the millisecond stamp the §10
  contract guarantees.
- `SteamLog/` collects only `<Steam>\logs\` files named in
  `AetherDesk/src-tauri/src/core/steam_logs.rs::SELECTED_STEAM_LOGS`, plus
  Steam's own `.previous.txt` session twin of each, and only when the file's
  mtime is within the last 12 hours. Lines are kept when their
  `[YYYY-MM-DD HH:MM:SS]` Steam stamp is within the window; a file without
  any stamp line is included whole, tail-capped at 4 MiB. Steam stamps are
  second-precision, so Desk's millisecond stamps are what order the two
  families against each other.

---

## Related

- [`AetherDesk/docs/ARCHITECTURE.md`](../AetherDesk/docs/ARCHITECTURE.md) —
  Desk module map, mutation ownership and settings repository.
- [`AetherDLL/docs/ARCHITECTURE.md`](../AetherDLL/docs/ARCHITECTURE.md) —
  AetherCore invariants.
- [`docs/glossary.md`](glossary.md) — shared vocabulary.
- [`docs/quality-gates.md`](quality-gates.md) — the gates that run the
  contract tests.
