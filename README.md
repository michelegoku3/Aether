# Aether

Aether is a Windows toolkit for Steam libraries. It installs Lua scripts and
manifests, manages backups and versions, and adds online/presence features.

It is made of two halves:

- **AetherDesk** — the desktop app (Tauri 2, React/TypeScript + Rust). This is
  the user-facing program.
- **AetherDLL** — C++ DLLs injected into Steam that apply the changes live.

## How to use Aether

1. Don't have any other lua tools installed inside Steam
2. Download the latest release of AetherDesk
3. Open the app and install AetherDLL from inside it

## Repository layout

| Path | Contents |
|---|---|
| `AetherDesk/` | Desktop app: `src/` (React/TS) and `src-tauri/` (Rust backend) |
| `AetherDLL/` | C++ DLLs, tests and protobuf schema |
| `Tools/` | Development scripts and test fixtures |
| `docs/` | Cross-cutting documentation (index in `docs/INDEX.md`) |
| `Licenses/` | Third-party licenses (`CREDITS.md`) |

## Building

Everything checks run identically locally and on CI.

```cmd
build_desk.cmd     :: AetherDesk: npm audit + verify + cargo clippy/audit/test
build_dll.cmd      :: AetherDLL: clang-format gate + cmake build + ctest + package
```

See `docs/quality-gates.md` for what each gate blocks, and
`AetherDLL/docs/quick-build.md` for the fast iteration loop.

## Documentation

- **`AGENTS.md`** — repository map and conventions (best starting point).
- **`docs/INDEX.md`** — index of every document.
- **`docs/shared_contracts.md`** — the AetherDesk ↔ AetherDLL contract.
- **`docs/quality-gates.md`** — build/quality gates.
- **`AetherDLL/docs/ARCHITECTURE.md`** — AetherCore architecture invariants.
