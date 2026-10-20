# Documentation index

Start with [`AGENTS.md`](../AGENTS.md) (canonical repository map and
conventions) and [`README.md`](../README.md) (what Aether is). Everything else
is listed below.

**Read it when** is the column that matters: it tells you which document
governs the change you are about to make.

## Reference — read these when you touch the related code

| Document | What it is | Read it when |
|---|---|---|
| [`docs/shared_contracts.md`](shared_contracts.md) | The AetherDesk ↔ AetherDLL contract: on-disk layout, backup/restore, quota, IPC keys, Steam-path rule. | You change a value or rule that both sides must agree on, or add/rename an IPC command. |
| [`docs/quality-gates.md`](quality-gates.md) | What every build check blocks and where it is configured, locally and on CI. | A gate fails, or you add one. |
| [`docs/glossary.md`](glossary.md) | Shared vocabulary: depot, GID, pin, depotcache, AetherData, presence. | A term in a document or a log line is unclear. |
| [`AetherDesk/docs/ARCHITECTURE.md`](../AetherDesk/docs/ARCHITECTURE.md) | Desk process model, module map, IPC/mutation/settings invariants, "where to add X". | You add or move a backend module, an IPC command, a pipeline or a view. |
| [`AetherDLL/docs/ARCHITECTURE.md`](../AetherDLL/docs/ARCHITECTURE.md) | AetherCore invariants: shared state, hook lifecycle, workers, persistence, presence. | You add state, a hook, a worker or a subsystem to the DLL. |
| [`AetherDLL/docs/quick-build.md`](../AetherDLL/docs/quick-build.md) | The fast AetherDLL loop (sync + Build All, pre-generated protobuf, ccache) and Desk build tips. | Builds feel slow or you touched `proto/`. |
| [`Tools/contract_fixtures/manifest/TABLE.md`](../Tools/contract_fixtures/manifest/TABLE.md) | The synthetic manifest-identity fixtures shared by the C++ and Rust tests. | You touch manifest identity parsing or its tests. |
| [`Licenses/CREDITS.md`](../Licenses/CREDITS.md) | Third-party projects Aether builds on and their licenses. | You add, remove or redistribute a bundled dependency. |

## Conventions

- Cross-cutting rules live in `docs/`; per-half architecture lives in
  `AetherDesk/docs/` and `AetherDLL/docs/`.
- Reference documents (contracts, architecture, guides) open with **what it is
  / read it when / not here** and close with **Related**; the entry points and
  `Licenses/CREDITS.md` keep their own shape.
- Names are kebab-case, except the long-standing uppercase pages (`README.md`,
  `AGENTS.md`, `docs/INDEX.md`, `ARCHITECTURE.md`, `TABLE.md`).
- Documents describe the current state only: no changelog, no batch or fix
  numbers, no dates of when something changed (see
  [`AGENTS.md`](../AGENTS.md#conventions-and-guardrails)).
- `npm run check:docs` (from `AetherDesk/`) verifies that every Markdown file is
  listed here and that every relative link and repository path resolves.
