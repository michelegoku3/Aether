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
| `Tools/` | Dev scripts: proto regen, DLL sync, version check, achievement decoder, docs check, test fixtures |
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

Module responsibilities, invariants and the "where to add X" recipes are in
`AetherDesk/docs/ARCHITECTURE.md`.

## AetherDLL layout (`AetherDLL/AetherCore/`)

- `core/` — `AetherCoreState`, hook manager/bootstrap, settings, ABI types.
- `hooks/` — `wire/` (protocol packets), `steamclient/` (ownership/license/
  persona), `ipc/`, `aetheronline/`, `license/`, `steamui/`.
- `network/`, `scripting/`, `credentials/`, `utils/`, `diagnostics/`,
  `inject/`, `security/`.

Other DLL targets: `AetherPayload/` (injected payload), `AetherXInput/`
(xinput proxy). `proto/steam_messages.proto` is the wire schema; the files in
`proto/generated/` are committed build outputs (see `Tools/regen_proto.cmd`).
The invariants behind these modules are in `AetherDLL/docs/ARCHITECTURE.md`.

## Task routing

Find the row that matches the change; it names the document that governs it,
the code that implements it and the check that proves it.

| Task | Read | Code | Verify |
|---|---|---|---|
| Add or rename an IPC command | `docs/shared_contracts.md` §7 | `src-tauri/src/commands/<area>.rs`, `main.rs`, `src/types/` | `cargo test ipc_contract` |
| Change a rule both halves must honor | `docs/shared_contracts.md` | the two "twins" listed in that section | `cargo test` + `build_dll.cmd` |
| Touch Lua pins, versions or the manifest resolver | `docs/shared_contracts.md` §2, §3, §5 | `manifest/pins/`, `manifest/resolver.rs`, `versioning/` | `cargo test manifest` |
| Change install/publish order | `docs/shared_contracts.md` §3 | `steam/compat.rs`, `core/game_mutations.rs` | `cargo test` |
| Change backups or restore | `docs/shared_contracts.md` §2 | `core/backup.rs`, `AetherCore/hooks/wire/ManifestRestore.cpp` | `cargo test backup` + `build_dll.cmd` |
| Add a store provider or source | `AetherDesk/docs/ARCHITECTURE.md` | `providers/`, `application/store/` | `npm run verify` + `cargo test` |
| Add or change a UI view, modal or hook | `AetherDesk/docs/ARCHITECTURE.md` | `src/views/`, `src/modals/`, `src/hooks/`, `src/style.css` | `npm run verify` |
| Change a setting | `AetherDesk/docs/ARCHITECTURE.md` (settings) | `core/settings*.rs`, `core/config_document.rs`, Settings UI | `cargo test` + `npm run verify` |
| Add shared mutable state to the DLL | `AetherDLL/docs/ARCHITECTURE.md` (core rule) | `AetherCore/core/AetherCoreState.h` | `build_dll.cmd` |
| Touch achievements or the Steam cache | `AetherDLL/docs/ARCHITECTURE.md` | `AetherCore/hooks/wire/Achievement*.cpp`, `Tools/achievement_decoder.py` | `build_dll.cmd` |
| Touch presence, online or UCO2/OFME | `docs/shared_contracts.md` §7.1 | `online/`, `AetherCore/hooks/aetheronline/` | `npm run verify` + `build_dll.cmd` |
| Work on Workshop manifests | `docs/shared_contracts.md` §1, §4 | `commands/workshop.rs`, `core/hubcap_update_monitor/` | `cargo test` |
| A build or lint gate fails | `docs/quality-gates.md` | the configuration named in the failing step | the failing command |
| Make DLL builds fast | `AetherDLL/docs/quick-build.md` | `Tools/sync_aetherdll.cmd`, `Tools/regen_proto.cmd` | `build_dll.cmd` |
| A term or log line is unclear | `docs/glossary.md` | — | — |
| Find any document | `docs/INDEX.md` | — | `npm run check:docs` |

## Build, test, lint

| Target | Full (gates) | Quick iteration |
|---|---|---|
| AetherDesk / frontend | `build_desk.cmd` or `cd AetherDesk && npm run verify` | `npm run build:fast`, `npm run typecheck` |
| AetherDesk / Rust | `cd AetherDesk/src-tauri && cargo test` | `cargo clippy --all-targets` |
| AetherDLL / C++ | `build_dll.cmd` (format + build + ctest) | `cmake --build --preset x64-Release` |

Outside an elevated shell the test harness needs
`AETHERDESK_NO_ADMIN_MANIFEST=1` (see `docs/quality-gates.md`).

CI is tag-only by design. `.github/workflows/build.yml` runs the release jobs
when a `desk-*` or `dll-*` tag is pushed; there is no pull-request or branch
CI. Aether has a single maintainer who runs the full gates locally before
tagging, so the workflow is the publishing barrier, not a review gate. Do not
add a PR workflow unless that working model changes.

## Documentation rules

- This file is the entry point; `docs/INDEX.md` lists every document with
  "read it when"; `docs/glossary.md` defines the shared vocabulary.
- Reference documents (contracts, architecture, guides) open with **what it is
  / read it when / not here** and close with **Related**; the entry points and
  `Licenses/CREDITS.md` keep their own shape.
- Cross-references between documents are relative Markdown links, not bare
  paths. Cited paths that start with a top-level folder are relative to the
  repository root (`check:docs` verifies them); the task-routing table uses
  module-relative paths for brevity.
- Names are kebab-case, except the long-standing uppercase pages (`README.md`,
  `AGENTS.md`, `docs/INDEX.md`, `ARCHITECTURE.md`, `TABLE.md`).
- Adding, renaming or deleting a document means updating `docs/INDEX.md` in the
  same commit. `npm run check:docs` (from `AetherDesk/`) verifies the index,
  every relative link and every cited repository path.

## Conventions and guardrails

- **Commit messages are a single line in `action(where): what changed` form.**
  Use a lowercase action (`feat`, `fix`, `perf`, `refactor`, `docs`, `test`,
  `build`, `chore`, `style`, `revert`), a lowercase scope, and a lowercase
  subject. Never add a body, bullet list or trailer below the subject.
- **Comments are English**, present tense, and explain *why*: invariants,
  contracts, ordering constraints, platform quirks.
- **Code and documents describe the current state only.** Never write change
  history: no "batch", "fix 2", "round 3", "step 4", no dates of when something
  changed ("2026-10-05"), no "was removed", "now uses", "legacy migration step".
  If a rule changes, rewrite the rule. Git history is the changelog.
- **`docs/changelog.md` is the one exception**: it records release history for the
  people who use AetherDesk and AetherDLL, and the maintainer writes it by hand at
  release time. Agents ignore it and never edit it unless the maintainer asks for a
  change to it in that conversation.
- **Do not hand-edit generated files**: `AetherDLL/proto/generated/**` is produced
  by `Tools/regen_proto.cmd`. Edit `proto/steam_messages.proto` instead.
- **IPC argument keys** are lowerCamelCase and derived from the Rust parameter
  names; no snake_case fallback (`docs/shared_contracts.md` §7).
- **The Steam path is resolved in the backend**, never passed by the client
  (`docs/shared_contracts.md` §8).
- **Do not commit build outputs** (`AetherDesk/dist/`, `target/`, `AetherDLL/out/`,
  `node_modules/` are ignored).
- When a shared rule in `docs/shared_contracts.md` changes, update **both**
  implementations, that page and the contract tests in the same commit.
