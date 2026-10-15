# Fast AetherDLL builds

Guide to the fast dev workflow for AetherDLL (replaces the legacy
`compilazione-veloce.md`, removed: same content, English filename for
discoverability).

Guide to the AI-agent-in-browser dev workflow: cutting build times from ~576 steps ("Rebuild All")
to a few seconds.

The recommended entry point is `build_dll.cmd` from the repository root:

```cmd
build_dll.cmd              :: full build: format gate + configure + build + ctest + package
build_dll.cmd /skipformat  :: skip the clang-format gate
build_dll.cmd /skiptests   :: skip the explicit ctest report
```

Quality gates (all enforced identically locally and on CI, see `docs/quality-gates.md`):

- `clang-format --dry-run --Werror` (style gate, `.clang-format` at `AetherDLL` root)
- `cmake --preset x64-Release` (`/W4 /WX`, clang-tidy when present, orphan-header check)
- `cmake --build --preset x64-Release` (unit tests compile as part of `ALL`)
- `ctest --test-dir out/build/x64-Release` (explicit report)
- `verify_version.ps1` (every DLL carries the `project(... VERSION x.y.z)` resource)

## The problem

1. Files downloaded from GitHub carry the commit time as timestamp; Windows
   copy-paste preserves it. Ninja compares timestamps: when a source looks
   "older" than the already-compiled `.obj`, **Build All recompiles nothing**
   even though the content differs.
2. That forced **Rebuild All**, which also wipes and recompiles
   protobuf/abseil/lua/minhook (~500 of 576 steps).

## The new workflow (3 steps)

1. **Replace** the downloaded files inside `AetherDLL\` via copy-paste
   **without deleting the folder** (the `out\` build cache must survive).
2. **Run `Tools\sync_aetherdll.cmd`** (double click). It compares content hashes
   and refreshes the timestamp **only of really changed files**.
3. In Visual Studio: **Build → Build All** (NEVER "Rebuild All"). Ninja
   recompiles only touched files + link: typically **10-60 seconds**.

## Pre-generated protobuf (default ON)

`proto/generated/steam_messages.pb.{h,cc}` are **generated once with protoc 25.3
and committed in the repo**. So the build no longer compiles `protoc` +
`libprotoc` (~120 steps) nor full `libprotobuf` (~70 steps): AetherCore links
only `libprotobuf-lite`, and with `EXCLUDE_FROM_ALL` ninja builds just the
libraries actually linked.

- **When you edit `proto/steam_messages.proto`**: run `Tools\regen_proto.cmd` to
  regenerate the files in `proto/generated/` and commit them together with the
  `.proto`. (Requires protoc 25.3; when missing, the script explains how to get it.)
- **Escape hatch**: configure with `-DAETHER_PROTO_PREGEN=OFF` to go back to the
  old behavior (compiled protoc and automatic regen on every `.proto` change).

Verified: the committed files compile and serialize correctly against the
protobuf-lite v25.3 runtime (round-trip test run).

## ccache / sccache (optional, recommended)

A *compiler launcher* caches every compiled object: even a "Rebuild All" becomes
near-instant on unchanged parts (the standard technique in CI and C++ teams).

1. Download ccache for Windows: <https://github.com/ccache/ccache/releases>
   (`windows-x86_64` zip), extract and put the folder in `PATH`.
2. Set once the environment variables (required with MSVC + PCH):
   ```
   setx CCACHE_SLOPPINESS "pch_defines,time_macros"
   setx CCACHE_DIR "%LOCALAPPDATA%\ccache"
   ```
3. Restart Visual Studio: CMake detects ccache automatically
   (`[AetherDLL] Compiler cache enabled: ...` in the configure output).
   To disable it: `AETHER_COMPILER_CACHE=OFF` cache variable.

One-time warning triage: `cmake --preset x64-Release -DAETHER_WARNINGS_AS_ERRORS=OFF`
shows the whole `/W4` backlog in one pass; fix it, then go back to the default
`/WX` gate.

## Expected numbers

| Scenario | Build steps |
|---|---|
| Before (Rebuild All) | 576 |
| Full rebuild without protoc/libprotoc/libprotobuf | ~330-390 |
| Full rebuild **with ccache** (warm cache) | ~330-390 but almost all cached |
| **Normal workflow: sync + Build All** | **only changed files + link** |

## Note for AetherDesk (Tauri/Rust)

- The UI: use `npm run build:fast` (just `vite build`) during development;
  `tsc` (full typecheck, ever slower as code grows) stays in `npm run build`
  for packaging and in `npm run typecheck` whenever you want to verify it.
- The Rust backend is **a single crate**: every change recompiles the whole crate
  and relinks ~546 dependencies. For local trial builds consider
  `[profile.release] incremental = true` in `Cargo.toml` (slightly less optimized
  binaries, much faster rebuilds; remove it for releases).
- Keep `src-tauri\target\` and `node_modules\` **out** of the folders you
  replace, otherwise every replacement restarts from zero (10-20 minutes).
- Add a Windows Defender exclusion for the project folder and for `target\`:
  on Windows it is one of the most common slowdowns.
