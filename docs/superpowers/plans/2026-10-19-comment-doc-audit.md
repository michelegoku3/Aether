# Comment & Documentation Audit — Implementation Plan

> **For agentic workers:** This plan is a content audit, not a feature build:
> there is no test to write per task. Steps use checkbox (`- [ ]`) syntax for
> tracking. Each wave ends with a commit and a lightweight verification.

**Goal:** Make every first-party comment accurate, current, English and
concise, and give the repository a documentation entry map that lets an AI or a
new human find what exists and where, without changing a single line of code.

**Architecture:** Six waves, ordered so the repository is reviewable and
interruptible: (1) frontend TypeScript, (2) Rust backend excluding tests,
(3) Rust tests, (4) C++ AetherCore by subsystem, (5) remaining C++ DLLs and
common, (6) documentation. Each wave is one commit; comment edits never
touch code.

**Tech Stack:** TypeScript/React (Vite), Rust/Tauri, C++/CMake, Markdown.

**Spec:** This plan itself (approved in session on 2026-10-19); the decisions
are recorded under "Rules" below.

## Global Constraints

- **Comments only.** No code, string, or non-comment line changes. No
  reflowing of non-comment lines. Keep every `// SAFETY:` block.
- **Language:** English, present tense or imperative, no first person, no
  dates, no changelog narrative.
- **Trim:** keep the *why*, invariants, contracts, ordering constraints and
  platform quirks; cut history, restated code and duplicated explanation.
- **Doubts are kept and flagged**, never silently deleted.
- **Excluded paths:** `AetherDLL/proto/generated`, `out/`, `build/`, `target/`,
  `node_modules/`, `_deps`, `ExternalTools`, `Licenses/`, binary fixtures,
  any file carrying a third-party license header.
- **Verification:** lightweight per wave (compile/lint/typecheck); full suite
  (`build_desk.cmd` / `npm run verify`, `cargo test`, `ctest`) at the end.
- **Delivery:** worktree `comment-audit`, branch `chore/comment-audit`, one
  commit per wave.

## Rules applied to every comment

Remove or fix:
1. comments that contradict the code (renames, old behavior, wrong paths,
   wrong constants/quota values, removed modules);
2. dead references to files, commands or config keys that no longer exist;
3. history/changelog inside code (dates, "previously", "batch 1/2",
   "quick-win", "invasive batch");
4. restatement of obvious code; commented-out code blocks;
5. Italian text -> English;
6. stale `TODO`/`FIXME`/`XXX`/`HACK` -> actionable note, or removed.

Keep and shorten: rationale, invariants, contracts, `// SAFETY:`, ordering
constraints, Steam/ABI quirks. Reduce multi-paragraph headers to a short
summary, with a pointer to documentation when the detail already lives there.

## Review Focus

The failure modes most likely to bite (each handled by the rule it names):

1. **Silently dropping a comment that encodes an invariant** — when in doubt,
   keep and mark, do not delete (Rule 4 and Global Constraint "doubts kept").
2. **A comment fixed against the wrong reading of the code** — verify the
   claim in the code before rewriting it (Rule 1).
3. **Accidentally editing code while editing a comment** — every wave's diff
   is inspected so that changes land only on comment lines (Global Constraint).
4. **Breaking a tool-driven comment** — `// SAFETY:`, `// @ts-expect-error`,
   `#![...]`, doc-test links, and clang-format-relevant markers are preserved
   (Global Constraint).
5. **Documentation that repeats the code instead of pointing to it** — the map
   describes where things are and why, then links (Wave 6 rules).

---

## Wave 1: Frontend `AetherDesk/src`

**Files:** ~133 `.ts`/`.tsx` under `AetherDesk/src` (views, modals, hooks, ui,
util, app, layout, types, test, constants).

- [ ] Inventory every frontend file and its comments.
- [ ] Apply the Rules; verify each questionable claim against the code.
- [ ] Lightweight check: `npx tsc --noEmit` + `npm run lint:ci`.
- [ ] Inspect the diff: changes only on comment lines.
- [ ] Commit: `docs(frontend): audit and normalize comments`.

## Wave 2: Rust backend `AetherDesk/src-tauri/src` (excluding `tests/`)

**Files:** ~171 `.rs` files (commands, core, manifest, online, providers,
steam, versioning, crack, updater, store, application, external_tools,
game_info, steamless, util, local).

- [ ] Inventory and apply the Rules; verify claims.
- [ ] Lightweight check: `cargo clippy --all-targets -- -D warnings` (with the
      test env from `docs/quality-gates.md` when running tests later).
- [ ] Inspect the diff; commit: `docs(backend): audit and normalize comments`.

## Wave 3: Rust tests `AetherDesk/src-tauri/src/tests`

**Files:** 37 files. Test comments often narrate intent; keep the intent
sentence, drop the narrative.

- [ ] Apply the Rules.
- [ ] Lightweight check: `cargo clippy`; commit:
      `docs(backend-tests): audit and normalize comments`.

## Wave 4: `AetherDLL/AetherCore` (C++)

**Sub-blocks in order:** `hooks/wire` (41), `hooks/steamclient` (10),
`core` (30), `network` (14), `scripting` (10), `utils` (37), then the rest
(`credentials`, `diagnostics`, `hooks/ipc`, `hooks/license`, `hooks/steamui`,
`hooks/aetheronline`, `inject`, `security`).

- [ ] Per sub-block: apply the Rules, verify ABI/wire claims carefully.
- [ ] Lightweight check: configure + build (`cmake --preset x64-Release`),
      then `ctest` if available.
- [ ] Commit per sub-block (or one wave commit if diffs stay small).

## Wave 5: `AetherDLL/AetherPayload`, `AetherXInput`, `tests`, `common`

- [ ] Apply the Rules; lightweight build check; commit.

## Wave 6: Documentation

- [ ] `AGENTS.md` (new): repository map per module, conventions, build/test/lint
      commands, guardrails (comments in English, no edits to generated files).
- [ ] `README.md`: from 5 lines to an orientation (what Aether is, components,
      quick start, pointers).
- [ ] `docs/INDEX.md` (new): every doc, one line each, and when to read it.
- [ ] Move `AetherDLL/docs/04-showonline-plan.md` and
      `05-showonline-suffix-plan.md` to `docs/archive/`; remove from active index.
- [ ] Cross-link `shared_contracts.md`, `quality-gates.md`, `ARCHITECTURE.md`
      as reference; fix broken references (e.g. the missing
      `docs/03-presence-identity-plan.md`).
- [ ] Commit: `docs(repo): add entry map, index and archive stale plans`.

## Final verification

- [ ] `build_desk.cmd` / `npm run verify`, `cargo test`, `ctest` — report real
      output.
- [ ] Whole-branch diff review: confirm no code changed.
- [ ] Report results and leave the branch for review/merge.
