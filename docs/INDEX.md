# Documentation index

Start with `AGENTS.md` (repository map and conventions) and `README.md`
(what Aether is and how to build it). Everything else is listed below.

## Reference — read these when you touch the related code

| Document | What it is | Read it when |
|---|---|---|
| `docs/shared_contracts.md` | The AetherDesk ↔ AetherDLL contract: on-disk layout, backup/restore rules, quota, and the Desk→frontend IPC key/Steam-path rules. | You change a value or rule that both sides must agree on, or add/rename an IPC command. |
| `docs/quality-gates.md` | What every build check blocks and where it is configured, locally and on CI. | A build/test/lint gate fails, or you add one. |
| `AetherDLL/docs/ARCHITECTURE.md` | AetherCore invariants: shared state (`AetherCoreState`), hook lifecycle, workers, ownership rules. | You add state, a hook, a worker or a subsystem to the DLL. |
| `AetherDLL/docs/quick-build.md` | The fast AetherDLL dev loop (sync + Build All, pre-generated protobuf, ccache). | Builds feel slow or you touched `proto/`. |
| `Tools/contract_fixtures/manifest/TABLE.md` | The synthetic manifest-identity fixtures shared by the C++ and Rust tests. | You touch manifest identity parsing or its tests. |
| `Licenses/CREDITS.md` | Third-party projects Aether builds on and their licenses. | You add, remove or redistribute a bundled dependency. |

## Process

| Document | What it is |
|---|---|
| `docs/superpowers/plans/2026-10-19-comment-doc-audit.md` | The comment/documentation audit plan this change implements. |
