# Quality gates — what the build checks and where

Single rule: **every check runs identically locally and on GitHub**. The
release workflow (`.github/workflows/build.yml`) adds nothing that `build.cmd`
or "Build All" in Visual Studio don't already do; it only serves as the final
barrier before publishing.

## AetherDesk — `build.cmd` (and the `release-desk` job)

| Step | Command | What it blocks | Where it is configured |
|---|---|---|---|
| 3 | `npm audit --audit-level=high` | high/critical CVEs in npm dependencies, without touching lockfile or working tree | — (`/skipaudit` to skip) |
| 4 | `npm run verify` → `check:tauri-versions` | major/minor mismatch between the Rust runtime, the JS API and the Tauri CLI | `scripts/check_tauri_versions.mjs` |
| 4 | `npm run verify` → build | TS type errors (`strict`, `noUnusedLocals/Parameters`) | `tsconfig.json` |
| 4 | `npm run verify` → `lint:ci` | `error`-level ESLint rules (hooks, unused imports, fallthrough) | `eslint.config.js` |
| 4 | `npm run verify` → `test:ci` | React/TypeScript regressions and coverage thresholds | `vitest.config.ts` |
| 4 | `npm run verify` → `check:wiring` | `.tsx/.ts` files never imported from `main.tsx` | `scripts/check_frontend_wiring.mjs` |
| 4 | `npm run verify` → `check:unused` (**knip**) | never-imported exports/types, unused or undeclared npm dependencies | `knip.json`, `scripts/check_unused.mjs` |
| 5 | `cargo clippy --all-targets -- -D warnings` | every rustc/clippy warning: dead code, unused imports, `unsafe` without `// SAFETY:`, locks held across `.await`, redundant clones, cast truncations | `Cargo.toml` → `[lints.rust]`, `[lints.clippy]` |
| 6 | `cargo audit` | known CVEs in crates (RustSec database) | — (`/skipaudit`) |
| 6 | `cargo machete` | dependencies in `Cargo.toml` never used | — |
| 7 | `cargo test` | unit tests + IPC contract tests | `src/tests/` (`/skiptests`) |

`cargo-audit` and `cargo-machete` are installed automatically the first time.

### How to read a failure

- **clippy**: the message names the lint (`clippy::xxx`) plus a link. When the lint
  is systematic noise, disable it **in `Cargo.toml` with a comment saying
  why**, not with scattered `#[allow]`s. A local `#[allow]` is acceptable only
  with a comment on the line.
- **npm audit**: explicitly update the direct dependency and regenerate the
  lockfile; the build never runs `npm audit fix`, because a quality gate must
  not mutate dependencies or sources while verifying them.
- **knip – unused export**: the function/type is exported but no other file
  imports it → drop `export` (when used only there) or delete it.
- **knip – unused dependency**: remove it from `package.json`.
- **cargo audit**: update the crate (`cargo update -p <crate>`); when no fix
  exists, `cargo audit --ignore RUSTSEC-XXXX` with a reason in `build.cmd`.
- **cargo machete**: remove the line from `Cargo.toml`. False positives
  (crates used only via macros) are declared in
  `[package.metadata.cargo-machete] ignored = ["..."]`.

### Running Rust tests without elevation

The shipped exe requires administrator (Steam-folder writes), and that manifest
is embedded in every build artifact — including the `cargo test` harness, which
then fails with `ERROR_ELEVATION_REQUIRED` (os error 740) outside an elevated
shell. `src-tauri/build.rs` reads `AETHERDESK_NO_ADMIN_MANIFEST=1` to embed an
`asInvoker` manifest instead; `build.cmd` (step 7) and CI set it for tests only.
Release builds never set it, so the published exe is unchanged. Run locally with:

```powershell
cd AetherDesk/src-tauri
$env:AETHERDESK_NO_ADMIN_MANIFEST='1'; cargo test --locked
```

If a test genuinely needs privileges, mark it `#[ignore]` with a reason instead
of forcing elevation on the whole suite.

## AetherDLL — CMake (Visual Studio "Build All" and the `release-dll` job)

Everything lives in `AetherDLL/cmake/Quality.cmake` and applies to every target
via `aether_apply_quality(<target>)`.

| Check | What it blocks | Escape hatch |
|---|---|---|
| `/W4 /permissive- /Zc:preprocessor` | compiles at high warning level and conforming C++ (before: W1 default) | — |
| `/WX` | every MSVC warning is an error | `-DAETHER_WARNINGS_AS_ERRORS=OFF` (prints a WARNING at configure) |
| `/external:W0` | zero noise from protobuf/abseil/lua/MinHook | — |
| **clang-tidy** | static C++ bugs: use-after-move, narrowing, avoidable copies, missing locks, UB (CERT) | `-DAETHER_CLANG_TIDY=OFF`; checks in `AetherDLL/.clang-tidy` |
| `aether_check_sources` | `.cpp` in the target folder but not in the target | — |
| `aether_check_headers` | `.h/.inl` no project file includes | — |
| `aether_run_tests` (ALL) | `ctest` runs on every build; a red test = red build | `-DAETHER_BUILD_TESTS=OFF` |
| `verify_version.ps1` | DLL without version resource | — |

clang-tidy activates on its own when it finds the binary (Visual Studio Installer →
*Individual components* → **"C++ Clang tools for Windows"**). When missing, the
configure says so clearly and carries on.

### First activation (one time only)

`/W4 /WX` and clippy applied to ~60k pre-existing lines will surface a backlog.
Recommended procedure, one component at a time:

```text
AetherDLL : cmake --preset x64-Release -DAETHER_WARNINGS_AS_ERRORS=OFF
            → read the warnings, fix them, then drop the flag (back to ON)
AetherDesk: cd AetherDesk/src-tauri && cargo clippy --all-targets --fix --allow-dirty
            → hand-fix what remains, then build.cmd
Frontend  : cd AetherDesk && npm install && npx knip
            → remove flagged exports/dependencies, then build.cmd
```

After this cleanup the baseline is zero and every new warning is attributable to
the commit introducing it.
