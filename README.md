# Aether

Aether is a Windows toolkit for Steam libraries. It installs Lua scripts and
manifests, manages backups and versions, adds online/presence features, etc.

It is made of two halves:

- **AetherDesk** — the desktop app (Tauri 2, React/TypeScript + Rust). This is
  the user-facing program.
- **AetherDLL** — C++ DLLs injected into Steam that apply the changes live.

## How to install Aether

1. Go into Steam folder
2. Deleted everything EXCEPT steam.exe and the folders (you WONT lose games, savegames, achievements)
3. Double click on steam.exe and wait it to finish
2. Download AetherDesk 1.4.3 from https://github.com/michelegoku3/Aether/releases/tag/desk-1.4.3
3. Extract AetherDesk folder on the desktop
4. Open AetherDesk.exe and exclude it from the AV
5. Go to Aether tab and install AetherDLL
6. Put your API key in the settings, scroll down to save

## Repository layout

Short version; the canonical map with module-level detail is
[`AGENTS.md`](AGENTS.md).

| Path | Contents |
|---|---|
| `AetherDesk/` | Desktop app: `src/` (React/TS) and `src-tauri/` (Rust backend) |
| `AetherDLL/` | C++ DLLs, tests and protobuf schema |
| `Tools/` | Development scripts and test fixtures |
| `docs/` | Cross-cutting documentation (index in [`docs/INDEX.md`](docs/INDEX.md)) |
| `Licenses/` | Third-party licenses ([`CREDITS.md`](Licenses/CREDITS.md)) |

## Building

Everything checks run identically locally and on CI.

```cmd
build_desk.cmd     :: AetherDesk: npm audit + verify + cargo clippy/audit/test
build_dll.cmd      :: AetherDLL: clang-format gate + cmake build + ctest + package
```

See [`docs/quality-gates.md`](docs/quality-gates.md) for what each gate blocks,
and [`AetherDLL/docs/quick-build.md`](AetherDLL/docs/quick-build.md) for the fast
iteration loop.

## Documentation

- **[`AGENTS.md`](AGENTS.md)** — repository map and conventions (best starting point).
- **[`docs/INDEX.md`](docs/INDEX.md)** — index of every document, with "read it when".
- **[`docs/shared_contracts.md`](docs/shared_contracts.md)** — the AetherDesk ↔ AetherDLL contract.
- **[`docs/quality-gates.md`](docs/quality-gates.md)** — build and quality gates.
- **[`docs/glossary.md`](docs/glossary.md)** — shared vocabulary.
- **[`AetherDesk/docs/ARCHITECTURE.md`](AetherDesk/docs/ARCHITECTURE.md)** — Desk module map and invariants.
- **[`AetherDLL/docs/ARCHITECTURE.md`](AetherDLL/docs/ARCHITECTURE.md)** — AetherCore invariants.

