# Quality gates — what the build checks and where

**What it is:** every check the build and CI run, what each one blocks and
where it is configured.

**Read it when:** a gate fails, you add or change a check, or a local run
disagrees with CI.

**Not here:** the commands for day-to-day work (see
[`AGENTS.md`](../AGENTS.md#build-test-lint)) or the fast iteration loop (see
[`AetherDLL/docs/quick-build.md`](../AetherDLL/docs/quick-build.md)).

Single rule: **every check runs identically locally and on GitHub**, with two
deliberate exceptions in the desk build: it runs the frontend tests without
coverage (the thresholds stay a CI gate) and reuses the local advisory database
for up to a week. The release workflow (`.github/workflows/build.yml`) otherwise
adds nothing that `build_desk.cmd` or "Build All" in Visual Studio don't already
do; it only serves as the final barrier before publishing.


## AetherDesk — `build_desk.cmd` (and the `release-desk` job)

| Step | Command | What it blocks | Where it is configured |
|---|---|---|---|
| 3 | `npm audit --audit-level=high` | high/critical CVEs in npm dependencies, without touching lockfile or working tree | — (`/skipaudit` to skip) |
| 4 | `npm run build` | TS type errors (`strict`, `noUnusedLocals/Parameters`) and a frontend bundle that does not compile | `tsconfig.json` |
| 4 | `npm run verify:build` → `check:tauri-versions` | major/minor mismatch between the Rust runtime, the JS API and the Tauri CLI | `scripts/check_tauri_versions.mjs` |
| 4 | `npm run verify:build` → `format:check` | Prettier drift in `src/`, `src-tauri/` JSON and the root JSON files | `.prettierrc` |
| 4 | `npm run verify:build` → `lint:ci` | `error`-level ESLint rules (hooks, unused imports, fallthrough) | `eslint.config.js` |
| 4 | `npm run verify:build` → `test:run` | React/TypeScript regressions. Coverage thresholds are a CI gate: `npm run verify` runs `test:ci` (`vitest --coverage`) | `vitest.config.ts` |
| 4 | `npm run verify:build` → `check:wiring` | `.tsx/.ts` files never imported from `main.tsx` | `scripts/check_frontend_wiring.mjs` |
| 4 | `npm run verify:build` → `check:docs` | a Markdown file no one links from `docs/INDEX.md`, a dangling relative link or heading anchor, a cited repository path that no longer exists | `Tools/check_docs.mjs` |
| 4 | `npm run verify:build` → `check:unused` (**knip**) | never-imported exports/types, unused or undeclared npm dependencies | `knip.json`, `scripts/check_unused.mjs` |
| 4 | `cargo clippy --all-targets -- -D warnings` | every rustc/clippy warning: dead code, unused imports, `unsafe` without `// SAFETY:`, locks held across `.await`, redundant clones, cast truncations | `Cargo.toml` → `[lints.rust]`, `[lints.clippy]` |
| 4 | `cargo test` | unit tests + IPC contract tests | `src/tests/` (`/skiptests`) |
| 5 | `cargo audit` | known CVEs in crates (RustSec database) | — (`/skipaudit`) |
| 5 | `cargo machete` | dependencies in `Cargo.toml` never used | — |

`npm run build` writes `dist/` first because `cargo test` compiles
`generate_context!`, which embeds it; then `verify:build` (everything `verify`
does, minus the repeated build and minus coverage) runs in parallel with
`cargo fmt`/`clippy`/`test`. The two halves keep separate logs.

`cargo audit` passes `--no-fetch` while the local advisory database is younger
than a week, so the network round trip happens weekly instead of on every build.

`cargo-audit` and `cargo-machete` are installed automatically the first time.

### How to read a failure

- **clippy**: the message names the lint (`clippy::xxx`) plus a link. When the lint
  is systematic noise, disable it **in `Cargo.toml` with a comment saying
  why**, not with scattered `#[allow]`s. A local `#[allow]` is acceptable only
  with a comment on the line.
- **npm audit**: explicitly update the direct dependency and regenerate the
  lockfile; the build never runs `npm audit fix`, because a quality gate must
  not mutate dependencies or sources while verifying them.
- **docs**: the message names the document and the dangling reference. Link the
  new document from `docs/INDEX.md`, or fix the path/anchor in the document.
- **knip – unused export**: the function/type is exported but no other file
  imports it → drop `export` (when used only there) or delete it.
- **knip – unused dependency**: remove it from `package.json`.
- **cargo audit**: update the crate (`cargo update -p <crate>`); when no fix
  exists, `cargo audit --ignore RUSTSEC-XXXX` with a reason in `build_desk.cmd`.
- **cargo machete**: remove the line from `Cargo.toml`. False positives
  (crates used only via macros) are declared in
  `[package.metadata.cargo-machete] ignored = ["..."]`.

### Running Rust tests without elevation

The shipped exe requires administrator (Steam-folder writes), and that manifest
is embedded in every build artifact — including the `cargo test` harness, which
then fails with `ERROR_ELEVATION_REQUIRED` (os error 740) outside an elevated
shell. `src-tauri/build.rs` reads `AETHERDESK_NO_ADMIN_MANIFEST=1` to embed an
`asInvoker` manifest instead; `build_desk.cmd` (step 4) and CI set it for tests only.
Release builds never set it, so the published exe is unchanged. Run locally with:

```powershell
cd AetherDesk/src-tauri
$env:AETHERDESK_NO_ADMIN_MANIFEST='1'; cargo test --locked
```

If a test genuinely needs privileges, mark it `#[ignore]` with a reason instead
of forcing elevation on the whole suite.

### Defender-sensitive tests (os error 225)

Some tests build a fake unpacker and copy executable bytes (the Steamless
runner: `valid_successful_output_replaces_live_file_and_preserves_exact_original`,
`wrong_architecture_is_rejected_before_creating_backup`,
`output_with_an_open_writer_is_not_accepted_as_stable`). Windows Defender's
real-time scan detects the test-produced PE files in `%TEMP%` and refuses to read
them with `os error 225` (`ERROR_VIRUS_INFECTED`), failing the tests and blocking
the build even though nothing is wrong.

A process exclusion is not enough: the detection is on the file itself. Point the
test harness at a dedicated temp directory and exclude that directory once, in an
**admin** shell:

```powershell
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Temp\AetherDesk-tests" | Out-Null
Add-MpPreference -ExclusionPath "$env:LOCALAPPDATA\Temp\AetherDesk-tests"
```

`build_desk.cmd` (step 4) sets `TEMP`/`TMP` to that directory for `cargo test`;
do the same when running the tests by hand:

```powershell
cd AetherDesk/src-tauri
$env:AETHERDESK_NO_ADMIN_MANIFEST='1'
$env:TEMP="$env:LOCALAPPDATA\Temp\AetherDesk-tests"; $env:TMP=$env:TEMP
cargo test --locked
```

This keeps the runner tests enabled locally and in CI, instead of `#[ignore]`-ing
them (which would also disable them where they pass). Undo the exclusion with
`Remove-MpPreference -ExclusionPath "$env:LOCALAPPDATA\Temp\AetherDesk-tests"`.

## AetherDLL — CMake (Visual Studio "Build All" and the `release-dll` job)

Everything lives in `AetherDLL/cmake/Quality.cmake` and applies to every target
via `aether_apply_quality(<target>)`.

| Check | What it blocks | Escape hatch |
|---|---|---|
| `/W4 /permissive- /Zc:preprocessor` | compiles at high warning level and conforming C++ | — |
| `/WX` | every MSVC warning is an error | `-DAETHER_WARNINGS_AS_ERRORS=OFF` (prints a WARNING at configure) |
| `/external:W0` | zero noise from protobuf/abseil/lua/MinHook | — |
| **clang-tidy** | static C++ bugs: use-after-move, narrowing, avoidable copies, missing locks, UB (CERT) | `-DAETHER_CLANG_TIDY=OFF` (prints a STATUS at configure); checks in `AetherDLL/.clang-tidy` |
| `aether_check_sources` | `.cpp` in the target folder but not in the target | — |
| `aether_check_headers` | `.h/.inl` no project file includes | — |
| `aether_run_tests` (ALL) | `ctest` runs on every build; a red test = red build | `-DAETHER_BUILD_TESTS=OFF` |
| `verify_version.ps1` | DLL without version resource | — |

clang-tidy is **required** by the `x64-Release` preset: the configure fails when
the binary is missing, so CI cannot silently ship without static analysis.
Install it from the Visual Studio Installer (*Individual components* ->
**"C++ Clang tools for Windows"**) or LLVM. `-DAETHER_CLANG_TIDY=OFF` is the
explicit opt-out; `-DAETHER_CLANG_TIDY=AUTO` skips it with a visible message
during one-off local setup.

### Turning a warning backlog into a list

When one change adds many warnings at once, the `-D` escape hatch above shows
the whole set in a single pass: fix it, then return to the default `/WX`.

```text
AetherDLL : cmake --preset x64-Release -DAETHER_WARNINGS_AS_ERRORS=OFF
            → read the warnings, fix them, then drop the flag (back to ON)
AetherDesk: cd AetherDesk/src-tauri && cargo clippy --all-targets --fix --allow-dirty
            → hand-fix what remains, then build_desk.cmd
Frontend  : cd AetherDesk && npm install && npx knip
            → remove flagged exports/dependencies, then build_desk.cmd
```

After this cleanup the baseline is zero and every new warning is attributable to
the commit introducing it.

## Related

- [`docs/INDEX.md`](INDEX.md) — every document, with "read it when".
- [`AGENTS.md`](../AGENTS.md#build-test-lint) — build commands and conventions.
- [`AetherDLL/docs/quick-build.md`](../AetherDLL/docs/quick-build.md) — the fast
  iteration loop.
