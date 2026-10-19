# Aether — repository guide

Aether is a Windows toolkit for Steam libraries. It has two halves that run in
different processes yet share one on-disk contract:

- **AetherDesk** — a Tauri 2 desktop app (React/TypeScript UI + Rust backend)
  that installs Lua scripts and manifests, manages backups, versions and
  updates.
- **AetherDLL** — C++ DLLs injected into Steam that apply the Lua/manifest
  changes live, mirror achievements and drive the online/presence features.

This file is the entry point. It tells you what exists, where, and how to build
and check it. Cross-cutting rules live in `docs/` (see `docs/INDEX.md`).

## Repository map

| Path | Contents |
|---|---|
| `AetherDesk/` | Tauri desktop app: `src/` (React/TS), `src-tauri/src/` (Rust backend) |
| `AetherDLL/` | C++ components: `AetherCore/` (main DLL), `AetherPayload/`, `AetherXInput/`, `tests/`, `proto/`, `cmake/` |
| `Tools/` | Dev scripts: proto regen, DLL sync, version check, achievement decoder, test fixtures |
| `docs/` | Cross-cutting documentation — start at `docs/INDEX.md` |
| `Licenses/` | Third-party licenses and `CREDITS.md` |
| `build_desk.cmd`, `build_dll.cmd` | The single build entry points (same checks as CI) |

## AetherDesk layout (`AetherDesk/`)

Frontend — `AetherDesk/src/`:

- `views/` (`Home`, `Library`, `Store`, `Log`, `Aether`, plus `views/settings/`,
  `views/store/`), `modals/`, `hooks/`, `ui/`, `app/`, `layout/`, `util/`,
  `types/`, `constants/`.

Rust backend — `AetherDesk/src-tauri/src/`:

- `commands/` — the `#[tauri::command]` IPC surface (one module per area).
- `core/` — settings, paths, backups, state, game mutations, logger.
- `manifest/`, `steam/`, `online/`, `providers/`, `versioning/`, `updater/`,
  `store/`, `application/`, `crack/`, `steamless/`, `external_tools/`,
  `game_info/`, `util/`, `local/`.
- `tests/` — Rust integration tests (see the module doc in `tests/mod.rs`).

## AetherDLL layout (`AetherDLL/AetherCore/`)

- `core/` — `AetherCoreState`, hook manager/bootstrap, settings, ABI types.
- `hooks/` — `wire/` (protocol packets), `steamclient/` (ownership/license/
  persona), `ipc/`, `aetheronline/`, `license/`, `steamui/`.
- `network/`, `scripting/`, `credentials/`, `utils/`, `diagnostics/`,
  `inject/`, `security/`.

Other DLL targets: `AetherPayload/` (injected payload), `AetherXInput/`
(xinput proxy). `proto/steam_messages.proto` is the wire schema; the files in
`proto/generated/` are committed build outputs (see `Tools/regen_proto.cmd`).

## Where to look for…

| Task | Read |
|---|---|
| Desk ↔ DLL contracts (backups, manifests, quota, IPC keys, Steam path) | `docs/shared_contracts.md` |
| Build and quality gates (local == CI) | `docs/quality-gates.md` |
| AetherCore architecture invariants (shared state, hooks, workers) | `AetherDLL/docs/ARCHITECTURE.md` |
| Fast AetherDLL dev loop | `AetherDLL/docs/quick-build.md` |
| Full documentation index | `docs/INDEX.md` |

## Build, test, lint

| Target | Full (gates) | Quick iteration |
|---|---|---|
| AetherDesk / frontend | `build_desk.cmd` or `cd AetherDesk && npm run verify` | `npm run build:fast`, `npm run typecheck` |
| AetherDesk / Rust | `cd AetherDesk/src-tauri && cargo test` | `cargo clippy --all-targets` |
| AetherDLL / C++ | `build_dll.cmd` (format + build + ctest) | `cmake --build --preset x64-Release` |

## Conventions and guardrails

- **Comments are English**, present tense, and explain *why*: invariants,
  contracts, ordering constraints, platform quirks. No changelog or dates in
  code.
- **Do not hand-edit generated files**: `AetherDLL/proto/generated/**` is produced
  by `Tools/regen_proto.cmd`. Edit `proto/steam_messages.proto` instead.
- **IPC argument keys** are lowerCamelCase and derived from the Rust parameter
  names; no snake_case fallback (`docs/shared_contracts.md` §7).
- **The Steam path is resolved in the backend**, never passed by the client
  (`docs/shared_contracts.md` §8).
- **Do not commit build outputs** (`AetherDesk/dist/`, `target/`, `AetherDLL/out/`,
  `node_modules/` are ignored).
- When a shared rule in `docs/shared_contracts.md` changes, update **both**
  implementations and that page in the same commit.
