# AetherDesk architecture

**What it is:** the process model, the module map and the invariants of the
AetherDesk half (React UI + Rust backend).

**Read it when:** you add or move a backend module, an IPC command, a pipeline
that writes a game, or a UI view; or you need to know which module owns a
piece of state.

**Not here:** the rules both halves must honor
([`docs/shared_contracts.md`](../../docs/shared_contracts.md)), the build gates
([`docs/quality-gates.md`](../../docs/quality-gates.md)), the AetherCore
invariants
([`AetherDLL/docs/ARCHITECTURE.md`](../../AetherDLL/docs/ARCHITECTURE.md)).

## Process model

- Every mutation of disk, Steam or the network happens in the Rust backend.
  The React UI only invokes commands and renders what comes back: it never
  touches the filesystem, Steam or a provider.
- A module in `commands/` is a thin IPC adapter: resolve the Steam path,
  validate inputs, call a domain module, map the error to `String`. Business
  logic lives in the domain module, not in the command.
- Pipelines that write a game (install, version switch, local import, crack,
  online deploy) run under the per-game mutation guard described below.
- Blocking work never runs on the async runtime: filesystem, archive and ZIP
  work goes through `spawn_blocking`; network work goes through provider lanes
  and the Hubcap scheduler.

## Backend map (`AetherDesk/src-tauri/src/`)

| Module | Responsibility |
|---|---|
| `main.rs` | Bootstrap: plugins, window setup, and the `generate_handler!` list where every command is registered. |
| `commands/` | The `#[tauri::command]` IPC surface, one module per area. Glue only. |
| `core/` | Cross-cutting infrastructure: settings and their repository/journal, `AetherData` paths, `GameBackup`, per-game mutations, state IO, logger, migrations, library events, Steam process/monitor, secure storage (DPAPI). |
| `manifest/` | Lua pin model, parser, editor (`pins/`), local-first resolver, provider package extraction, manifest identity check, pin-refresh lane. |
| `steam/` | Steam installation: path resolution, ACF/library scan, compat install (`install_lua_and_manifest_files`), launch options, update guard, app names and store metadata. |
| `providers/` | Remote sources: Hubcap (auth, catalog, generation, packages), LuaTools (+ auth), Ryuu, oureveryday, the shared HTTP client and the `ManifestSource` trait. |
| `application/` | Store use cases: latest and specific version download, package completion, installation, source ranking, concurrency. Must not depend on `commands`. |
| `store/` | Store catalog model: search cache, aliases, mapping, normalization, DRM flags. |
| `versioning/` | Version switching: snapshots, saved builds, apply, Lua validation, remote sources (`steamdb`, `depotbox`). |
| `online/` | UCOnline2 engine: detect, config, deploy, revert, state, foreign-online inspection. Pure Rust, no Tauri. |
| `local/` | Local import pipeline: archives or folders routed into `stplug-in`, `depotcache` and the game folder. |
| `crack/` | Crack apply engine: archive extraction, destination mapping, backup, inventory, recovery. |
| `steamless/` | Steamless runner: tool location, execution, output validation. |
| `external_tools/` | Discovery and integrity checks for the tools bundled under `src-tauri/ExternalTools/`. |
| `updater/` | AetherDesk and AetherDLL updates from GitHub: version check, install, uninstall, restore. |
| `game_info/` | Per-game info and SteamDB links (cache, service, model). |
| `util/` | Validation, dialogs, browser opener, game resolver. |
| `tests/` | Crate-level integration tests; the placement rule is in `tests/mod.rs`. |

## Frontend map (`AetherDesk/src/`)

| Folder | Responsibility |
|---|---|
| `app/` | App shell and runtime hooks (`useDllRuntime`, startup coordination). |
| `layout/` | Window chrome, sidebar, main content. |
| `views/` | One per top-level screen: Home, Library, Store, Log, Aether, Settings (with `views/settings/` and `views/store/` sections). |
| `modals/` | Dialogs opened from views, one file per flow (game actions, version editor, online, cleanup, updates). |
| `hooks/` | Shared state and data hooks (`useLibraryGames`, `useSettings`, polling). |
| `ui/` | Reusable presentational pieces: icons, cards, images. |
| `types/`, `constants/`, `util/` | IPC payload types, constants, helpers. |
| `test/` | Vitest setup and Tauri mocks. |

## Invariants

### IPC

- Commands are declared in `commands/<area>.rs` and registered in
  `main.rs::generate_handler!`; a declared-but-unregistered command is dead.
- Argument keys are the Rust parameter names in lowerCamelCase; there is no
  snake_case fallback.
- The Steam path is resolved in the backend, never sent by the client.
- Payload shapes with more than one argument are `type` aliases in
  `src/types/`, not inline objects.
- `cargo test ipc_contract` enforces all of the above against the real sources.
  The full rule set is in
  [`docs/shared_contracts.md` §7 and §8](../../docs/shared_contracts.md#7-desk--frontend-ipc-contract-argument-keys).

### Settings and DLL configuration

- `SettingsManager::update` runs strict load → typed patch → normalization →
  commit under a mutex and the advisory `settings.lock`. The patch closure must
  be pure: no remote I/O, no recursive repository calls.
- IPC `save_settings` takes the form snapshot as `base` alongside `settings`
  and returns the effective snapshot. Fields unchanged versus `base` keep the
  current value; a same-field conflict returns `SETTINGS_CONFLICT` without
  sensitive values in the message.
- `settings.json` and `provider_credentials.dat` keep their formats.
  `settings.pending.json` is a journal (version, sanitized JSON, DPAPI
  ciphertext) published before both files and replayed idempotently before
  every repository read or mutation. `settings.last-good.json` holds an older
  pair in the same journal format, not in the plain settings format.
- A present journal is recovered; a lingering `.lock` file is not an abandoned
  journal, because lock ownership belongs to the OS.
- `load()` stays a read-only adapter with logged fallback; write commands and
  `get_settings` use strict loading. A parse or DPAPI error never authorizes
  saving the defaults.
- Every Desk editor of `aethercore.toml` goes through
  `core/config_document.rs`: lossless parser, section-aware keys, per-document
  lock, unique temp files. The legacy copy is updated only when present. The
  two TOML copies are not a multi-file transaction: a sync error is reported,
  never declared as success.
- Rust 1.89 is the minimum for `std::fs::File` advisory locks; no additional
  locking crate is introduced.

### Per-game mutations

- `core/game_mutations.rs` identifies the resource with the canonicalized Steam
  root plus AppID. Equivalent aliases converge; on Windows the key is
  case-insensitive. The map holds weak references so it cannot grow forever.
- Remote operations: `MutationPlan::prepare` before reading inputs or preparing
  the download, `commit_for` immediately before publication. No game lock is
  held during networking. The commit rechecks Desk generation, Lua/ACF/
  libraryfolders fingerprint and the current root; a concurrent change returns
  `GAME_STATE_CONFLICT` without publishing the Lua/ACF prepared on old state.
  Manifests already restored from the resolver are immutable and may stay
  cached.
- Synchronous operations: `try_acquire` returns `GAME_BUSY` instead of blocking
  the runtime. Async lanes may await the same game's mutex. Coordination is
  in-memory only: it creates no lock files and does not coordinate separate
  Desk processes.
- Lock order is game → depotcache/config-editor gate. Never re-acquire the same
  game from a helper: the post-install Store refresh starts after the main
  transaction releases, and the shared realignment receives the guard from its
  caller.
- When delegating to `spawn_blocking`, the worker receives the guard by value:
  cancelling the waiter must not free a lock while the worker is still writing.
  The external-tool and online-deploy wrappers follow the same rule.
- Local imports keep ownership during writes, policy update and the final
  backup. Bulk imports acquire the discovered AppIDs in order before touching
  files. Workshop manifest writers never rewrite the game's Lua/ACF and keep
  their own gate and cache identity contract.
- Steam, the DLL, manual editors and older Desk builds do not obey these locks.
  Fingerprint and pre-replacement rechecks detect observable changes but offer
  no atomic compare-and-swap against uncooperative external writers; blocking
  the Steam process is not an acceptable substitute.
- The coordinator is not a multi-file rollback journal and does not solve
  read-modify-write of global JSON cache or online state across AppIDs: those
  are distinct domains.

### Manifest and Lua writes

- Every consumer resolves manifests through `manifest/resolver.rs`; a private
  copy of the lookup order would drift from the completeness gate. The order,
  the identity rule and the "manifest first, Lua last" commit live in
  [`docs/shared_contracts.md` §2, §3 and §5](../../docs/shared_contracts.md#2-manifest-backuprestore-contract).
- Every Lua rewrite archives pre and post to `backup/<app_id>/lua/history/` and
  backs up the manifests it references.
- Lua is parsed as text and never executed. Malformed pins are a user-fixable
  state, reported with their line, not a retryable error.
- Workshop repair targets are direct children of `WorkshopItemsInstalled` with
  a positive decimal manifest, plus `WorkshopItemDetails`-only subscriptions
  with a positive `manifest` in the explicit repair. The background monitor
  processes only installed items. Nested `manifest_history` indices and `-1`
  legacy entries are not items; a details entry without a positive `manifest`
  is not a subscription target.
- Installed items use `latest_manifest` when it is newer than the installed
  manifest. An update asks Steam for the payload even when the old content
  directory exists, and the report marks it as an update rather than "already
  local".
- A Hubcap `404` is a data-availability answer, not proof that a mod is gone.
  The report calls it "not in Hubcap or unknown error", and the 24 h cache is
  keyed by `(item, manifest)` so a changed manifest retries immediately.
- Never delete or edit `appworkshop_*.acf` to unsubscribe. The file is per
  game, Steam rebuilds it, and only the Steam Workshop page unsubscribes. An
  empty Workshop content directory is not content.

### Diagnostics

The logger has one domain per concern: `settings` (prepared/applying/finalized/
conflict), `config` (operation/path/changed/error, never values), `mutations`
(begin/release with ID, operation, AppID/root, duration; busy/conflict).
**Release means the resource was freed, not that the operation succeeded**: the
outcome belongs to the caller's log line.

## Where to add…

| Change | Touch |
|---|---|
| An IPC command | `commands/<area>.rs` + register in `main.rs` + payload `type` in `src/types/` + `cargo test ipc_contract`. |
| A setting the DLL also reads | `core/settings_model.rs` + repository validation/default + the Settings UI + `docs/shared_contracts.md` + both readers. |
| A Desk-only setting | `core/settings_model.rs` + repository + Settings UI. |
| A store provider | `providers/<name>.rs` implementing `ManifestSource` + ranking in `application/store/source.rs` + tests. |
| A manifest consumer | `manifest/resolver.rs` (never a private lookup) + the completeness gate. |
| A modal or view | `src/modals/` or `src/views/` + a section in `style.css` + a Vitest test next to it. |
| A field shown in the Library | `steam/` (source of truth) → `commands/library.rs` → `src/types/` and the hook. |
| A bundled external tool | `external_tools/` discovery + `src-tauri/ExternalTools/` + `Licenses/CREDITS.md`. |

## Verification

| Target | Full gate | Quick loop |
|---|---|---|
| Frontend | `build_desk.cmd` or `cd AetherDesk && npm run verify` | `npm run typecheck`, `npx vitest run <file>` |
| Rust backend | `cd AetherDesk/src-tauri && cargo test` (set `AETHERDESK_NO_ADMIN_MANIFEST=1` outside an elevated shell) | `cargo clippy --all-targets` |

A change that touches a rule in `docs/shared_contracts.md` must update both
implementations, that page and the contract tests in the same commit.

## Related

- [`AGENTS.md`](../../AGENTS.md) — repository map and conventions.
- [`docs/shared_contracts.md`](../../docs/shared_contracts.md) — Desk ↔ DLL
  contract.
- [`docs/quality-gates.md`](../../docs/quality-gates.md) — what each gate
  blocks.
- [`docs/glossary.md`](../../docs/glossary.md) — shared vocabulary.
