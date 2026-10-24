# Changelog

## AetherDesk V1.5.0

### Library and install state

- Steam's install state is read properly now: a game is *installed*, *installing*
  or *not installed*, taken from the ACF `StateFlags` rather than from "the folder
  exists". A paused or cancelled first download no longer counts as installed.
- The library shows an **Installing** badge on the card and a fourth filter
  (all → installed → installing → not installed) with its own icon.
- The folder watcher now binds every library's `steamapps` and diffs an install
  projection, so a real Steam install, update or uninstall refreshes the library
  without pressing Refresh.
- Actions guard against Steam writing the game: Apply Crack, Remove and Online
  re-read live state (new `resolveLiveGame`) and refuse while an install is running.
- A local import reuses Steam's own folder name for a game Steam is still writing,
  and a directory whose watch cannot be bound is retried on its own timer instead
  of being counted as observed.
- Fixed: an installed `.lua` game was shown as coming from a nonexistent family
  share together with an "outdated" warning.

### Settings and configuration safety

- Saves are three-way. The Settings screen sends the baseline it loaded, the
  backend compares and answers `SETTINGS_CONFLICT` instead of overwriting, and the
  screen shows a "changed elsewhere" banner with a reload action.
- New `config_document.rs` edits TOML losslessly through `toml_edit` and detects
  external changes; a pending journal plus `settings.last-good` allow recovery.
- A corrupt or unreadable `settings.json` no longer dead-ends the app: the file and
  `provider_credentials.dat` are quarantined aside, the app starts from defaults,
  and save, reset and acknowledge keep working.
- New `get_settings_health` separates a fresh install from a corrupt fallback, and
  Settings shows a red banner in the latter case, cleared after a successful save.
- Defaults live in one place: the serde defaults are named constants shared with
  `impl Default`, so a fresh install and a migrated file cannot diverge.
- Typed error codes (`core/error.rs` with `coded`/`split_code`) replace text
  matching for `SETTINGS_CONFLICT`, `GAME_STATE_CONFLICT`, `GAME_BUSY`,
  `HUBCAP_KEY_REQUIRED_FOR_UPDATES` and friends.
- A failing `getSettings()` at startup is caught and logged instead of aborting
  hydration; appearance, update checks and the Steam-path warning still run.
- `AppSettings` field order now matches the Settings screen and a test pins the
  serialized key order.

### Games: crack, online, versioning, backups

- **Per-game mutation guard.** A new `game_mutations` module takes a lock per
  (canonical Steam root, AppID) with an optimistic plan, answering `GAME_BUSY` or
  `GAME_STATE_CONFLICT`. Store installs, crack applies, online enable/disable and
  local imports all go through it.
- **Crack is transactional.** A `crack_<app>.state.json` journal records
  had_original / applied / pending hashes, originals become immutable, staging must
  sit outside the game root, and a reapply refuses an original that changed — so a
  second crack never treats the previous crack as the original files.
- **Crack placement.** Counterpart-free files now land in the real content root:
  the game folder is descended when the `installdir` holds a single wrapper folder
  named like itself or containing a matching `.exe`, and descent stops when the
  `installdir` has its own `.exe`.
- **Visual-novel patches.** The installer is waited on and its exit status checked
  (polled every 50 ms, bounded at 180 s), and a run that applied nothing is reported
  as an error instead of a success.
- **Online (UCO2/OFME).** Enable publishes binaries atomically, and a failed first
  enable replays the journal backwards and removes its backup; a presence failure
  after a successful deploy is a warning, not an error. Disable works without a
  recovery journal (it asks for confirmation, removes UCO2 files, keeps originals
  and warns about integrity), survives lost journals and cross-drive libraries, and
  treats an empty leftover backup directory as a no-op.
- **Crack and UCO2 cannot coexist.** Apply Crack refuses while UCO2 files are
  present, and the OFME banner gained a Clean button that removes both file sets
  after confirmation and clears the online state.
- **Versioning.** A version change no longer inserts old depots when a specified
  manifest cannot be resolved — the snapshot reconstruction fails instead of
  returning partial pins. Bookmark toggles roll back if the backend refuses and
  report why, the progress listener is torn down on unmount, and the disabled Auto
  tab is marked as under maintenance with a badge.
- **Backups.** A Desk-modified Lua (hand edit, update toggle) is archived into
  `history/` only and never overwrites the pristine `lua/<app>.lua`; the store
  install reads the Lua before and after applying the update policy so the pristine
  copy survives; the import mirror is written atomically.
- `localconfig.vdf` is committed through a sibling temp file with `sync_all` and a
  rename, after a `.bak` copy, so an interrupted edit can no longer truncate the
  Steam client configuration.
- **Uninstall** deletes only byte-identical files from a SHA-256 allowlist instead
  of wiping the install root, so files the user put there survive.
- Bulk local import publishes manifests before any Lua, copies atomically, keeps a
  pristine copy per Lua, and reports per-file failures ("Warning: N items failed")
  instead of failing or succeeding as a whole.

### Store

- A failed search reports the failure: `useStoreSearch` exposes `error`, the view
  shows a "Search failed" banner, the stale grid is cleared and the "no games
  found" empty state is suppressed.
- New `providers::health` tracks which sources answered; an empty result from
  sources that never answered now says "Steam and Hubcap are unreachable" instead
  of "no games found".
- New `providers::hubcap::network` classifies DNS, reset, timeout and TLS faults,
  rejects HTML block pages before the ZIP parser, and explains ISP/DNS/firewall/VPN
  causes.
- The MOED/oureveryday source is gone: provider, dispatch and sentinel key removed,
  so latest and specific-version downloads always go through Hubcap.
- The Hubcap usage counter shows the daily game limit again (25, not 1500
  manifests) and its cached stats snapshot is invalidated on a successful
  generation, so the badge updates without a restart.
- The search cache is pruned: entries older than 14 days are dropped and the cache
  is capped at 200 queries, evicting the oldest.
- Store opens on "latest" instead of "upcoming", with a one-time migration that
  only rewrites the old default.
- The trending cache is warmed on the deferred startup task, so opening the store
  reads a fresh entry instead of waiting on Steam.

### Workshop

- New scoped `appworkshop_*.acf` parser: it reads only the direct children of
  `WorkshopItemsInstalled` / `WorkshopItemDetails`, so per-branch `manifest_history`
  indices are no longer mistaken for Workshop IDs and no quota is burned on them.
- Discovery only treats installed items with a positive manifest as repair targets;
  not-yet-installed subscriptions are a separate list and empty content folders are
  flagged.
- Repairs return one row per item (ready / deferred / failed / unavailable, reason,
  HTTP status, mod title, empty content) and open a dedicated report popup instead
  of a toast; a partially failed run says how many items still need another Repair.
- Terminal Hubcap 404s are remembered for 24 hours keyed by item and manifest, with
  pruning, and a local or cached manifest always wins over a remembered 404.
- `manifestGid` is serialized as a string, because GIDs exceed JavaScript's safe
  integer range.
- Manifests are staged only: the `steam://workshop/downloaditem` spawn, its
  `workshop_auto_download_content` setting, the download caps and the deferral are
  all removed.
- The commit gate no longer covers the Hubcap network call: generation runs outside
  the lock and only the cache write plus depotcache install hold it.

### Updates and the updater

- `--apply-update` validates the install root before writing anything (absolute, no
  links or reparse points, not a protected folder, contains the running
  `AetherDesk.exe`) and exits 1 when it does not match.
- The updater copy is staged in a fresh `run-<pid>-<n>` directory and its SHA-256 is
  verified against the running executable before spawn, and again right before.
- A valid install below a reparse point (a redirected Desktop or Documents) is
  accepted: only the path itself is checked, not its ancestors.
- `build_desk.cmd` and the updater refuse before the app exits, so a bad root is a
  message instead of a silent close.

### Startup and performance

- Startup is deferred: only critical migrations run before the UI accepts IPC;
  legacy cleanup, shell shortcuts and the Start Menu sync go to a blocking worker
  after the first frame, with one settings snapshot and per-step timings in a new
  startup log line.
- The Steam library cache is warmed from the App IDs the provider already scanned
  instead of walking every ACF again, and the DLL status read no longer triggers the
  network update check.
- Store, Library and Settings take an `active` prop, so cover preloads, trending
  top-ups, Denuvo enrichment and the Settings bootstrap only run for the visible tab
  and only on first open.
- Settings reads are cached per config directory keyed by mtime and size of
  `settings.json` and `provider_credentials.dat`; a hit skips the mutex, file lock,
  recovery probe and DPAPI, and any pending journal disables the cache.
- The Logs screen pauses polling entirely when the tab is hidden and keeps row
  identity stable, so appended lines no longer re-render the list.
- Typeahead renders 20 results at a time and grows on scroll or keyboard selection.
- Clear Cache now also drops the process-lifetime caches (store suggest and meta,
  GitHub releases, Hubcap key validation and usage), reports file and memory counts
  and asks the UI to drop cover and hero caches, rescan the library and clear store
  results.
- `build_desk.cmd` skips `npm ci` when the lockfile stamp matches and runs the gates
  in parallel.

### Security and robustness

- Content-Security-Policy enabled (it was `null`), pinned by a snapshot test that
  fails unless the change is reviewed.
- `reveal_in_file_manager` only accepts paths inside app data, the install root or a
  Steam library, canonicalized through the first existing ancestor.
- Theme, wallpaper and icon names are validated: separators, `..` and absolute names
  are rejected, so a name can no longer escape the config folder; SVG is no longer
  accepted as a background format.
- 7z entries are validated (absolute and `..` paths skipped) and published through
  temp + rename; ZIP extraction decrypts per entry, so encrypted archives expose
  their entry names without the password.
- Archive passwords no longer reach messages or logs, and failures are one short
  line ("password rejected" vs "invalid archive") with per-format causes only in the
  log.
- Wallpapers over 8 MiB are refused instead of being base64-loaded into the WebView
  on every appearance refresh.
- Log reads are bounded: `tail_lines` is clamped to 1..=10,000 and tails seek from
  EOF with a size cap instead of loading whole files.
- Defender exclusion failures map to fixed summaries; only the first 300 characters
  of a stderr line reach the log, with no paths in the user-facing text.
- File and folder pickers return an error instead of a silently lossy path, atomic
  temp names are built from the OS string (no U+FFFD collisions), and every atomic
  write goes through one `state_io::write_atomic`.
- The hand-written Base64 encoder is gone in favour of the `base64` crate.
- Frontend error handling is typed: `toErrorMessage` / `isUserCancellation` replace
  `catch (err: any)` across views and modals, and `shortenPath` truncates by code
  points so emoji are no longer cut in half.
- The LuaTools sign-in callback is bound to a per-flow 32-byte `state` carried inside
  `redirect_to`, on a `127.0.0.1` loopback literal; a foreign callback can no longer
  complete the sign-in.
- Only one instance runs per session, which matters because mutation locks are
  process-local.
- The unsaved-changes guard is installed in the right lifecycle phase, drops the
  pending action when a save fails instead of stacking dialogs, and long-running
  status messages are no longer wiped by the 6 s timeout.

### UI and quality of life

- Settings reorganized into Aether / Steam / Providers; provider panels share one
  group rhythm, Reset opens a Yes/No confirmation, the Browse button moved inside the
  Steam path field, and the Steam section toggle reads "Show every game you are
  playing to friends".
- Library banner gained the family-share controls and a note that Steam must restart
  for the spoofed owner to appear.
- Steamless, Steam and the sidebar: the Start/Restart label re-checks Steam every 2 s
  while visible, so closing Steam returns the button to Start.
- The app window can actually be closed (the Tauri capability was missing) and the
  webview receives its push events again (new `capabilities/default.json`).
- Devtools open only after Save and only when the saved opt-in is on.
- Dead UI removed: `SyncStatusModal`, the sync-status event and `types/sync.ts`; the
  Apply Crack tooltip is replaced by a click-time message; UCOnline2 errors are short
  sentences without searched paths.
- The uninstall dialog and the modal copy now say that user files are kept, and the
  Reset Steam path popup replaces the old uninstall "Steam clean" dialog.

### Build, quality, docs and tooling

- Full build gate: `cargo fmt --check`, Prettier, clippy with `-D warnings`, `cargo
  audit`, `cargo machete`, knip (`check:unused`, with a self-test that plants a dead
  export to prove it detects one), the frontend test suite and `check:docs`.
- Frontend test suite added (Vitest + jsdom + Testing Library with Tauri IPC mocks,
  about 20 files, v8 coverage thresholds) and wired into `npm run verify`.
- `check_docs.mjs` guards the documentation: index coverage, relative links and
  anchors, cited repository paths. `docs/INDEX.md`, `glossary.md` and
  `shared_contracts.md` were rewritten around it.
- Contract fixtures (`Tools/contract_fixtures/manifest/`, 2 valid + 7 malformed
  blobs plus a `TABLE.md` of verdicts) asserted by both the Rust and C++ validators.
- Secret scanning (gitleaks), a repo-wide rustfmt and Prettier normalisation, and
  `npm audit` turned from a mutating `npm audit fix` into a non-mutating
  `--audit-level=high` gate.
- `.gitattributes` forces CRLF for `.cmd`/`.bat`/`.ps1` (cmd mishandles LF after
  label jumps) and LF elsewhere so the Prettier gate stops failing on Windows.
- Test-coverage artifacts are no longer committed.
- `Tools/bump_version.cmd` / `bump_version.ps1`: interactive version bump with
  `X.Y.Z` validation, expected-match counting per file, `-DryRun` and write-back
  verification, covering all five Desk version carriers.
- Dependency work: `zip` 1 → 9, `windows-sys`, `getrandom`, `h2`, `rustls`,
  `rustls-webpki` re-resolved, `ctutils` 0.4.5 for RUSTSEC-2026-0385, `undici` and
  `zod` past their advisories, Tauri pinned at ~2.11 with a version check script.
- Documentation: new root `AGENTS.md` with task routing and conventions, an expanded
  `README.md`, `docs/quality-gates.md` (including the non-elevated
  `AETHERDESK_NO_ADMIN_MANIFEST=1` run and Defender `os error 225`), a 215-line
  updater testing guide, and CI documented as tag-only by design.
- All remaining Italian text was translated to English, comments and docs included.

### Internal (no behaviour change)

- Module splits: `store.rs` → `application/store/*`, `pins.rs` → `pins/*`,
  `hubcap_update_monitor.rs` → six modules, `library_events.rs` → three, `settings.rs`
  → `settings_model` + `settings_repository`, `crack/apply.rs` → hashing/io/paths/
  recovery, `store/service` → mapping/model.
- Frontend splits: `SettingsView.tsx` (892 lines) → settings hooks, `StoreView.tsx`
  → search bar / grid / download modal / hooks, `App.tsx` → `app/` hooks,
  `GameInfoModal` and `GameCover` → helper modules.
- HTTP clients come from one process-wide registry keyed by timeout and headers, so
  every provider and Steam metadata call shares a connection pool.
- Steamless output is drained into a 64 KiB ring buffer instead of `read_to_end`.
- Six clippy passes over the backend (checked casts, `let-else`, SAFETY comments,
  `sort_by_key`, `div_ceil`, lifetime elision, dead exports removed) plus a
  repository-wide lint policy in `Cargo.toml`.
- A cross-process lock file per game was removed: coordination is in-process only,
  and `docs/shared_contracts.md` says so.
- Legacy compatibility dropped: Roaming/ProgramFiles and old-settings adoption,
  legacy wallpaper and `aether.ico` paths, `-aetheronline`/`-showonline` launch
  options, and the `dwmapi.dll` proxy bookkeeping in the DLL installer.

## AetherDLL V1.0.0

### Ownership and presence

- Steam's ownership is now checked against a manifest owner Aether knows about. New
  `AppManifestOwner` strips `AppState.LastOwner` from spoofed apps at boot, after
  login and after any manifest rewrite, and `CheckAppOwnership` clears
  `borrowed`/`familyShared` when the answer belongs to a spoofed app.
- The strip is an opt-in policy driven by `[library] owner_banner*` settings, with a
  new `OwnerRecordPolicy.h` and an `aethercore\owner_state.txt` record of Aether's
  own writes, so only the apps Aether touched get corrected.
- `PlanOwnerAction` now drops `LastOwner` on any app Aether spoofs — including one
  Steam stamped itself, which can only render a bogus "From the library of" banner.
- New `SteamUIStylePatch` rewrites `steamui\css\sp.css` in place, size-preserving,
  to hide the orange legacy-sharing notice.
- Presence identity (the real app behind the 480 mask, showonline, UCO2 spoof) moved
  from three `AetherCoreState` atomics into `hooks/aetheronline/PresenceSession`,
  with a single publish plus snapshot and an `EndSession` when the running-game count
  reaches zero.
- Library enumeration was extracted into `utils/SteamLibraryPaths.h`.

### Injection and payload integrity

- A payload whose file version differs from the core is refused, `VM_READ` is
  dropped from the mapping rights, and the payload SHA-256 is logged on every
  inject, so "injected" means the remote DLL is actually loaded.
- Child injection from `CreateProcessHooks` now requires the same payload version
  match as `MaybeInject`; `PayloadVersionMatches` moved out of `OnlinePayload` into
  `PayloadAttest` so both paths share it.
- `RemoteLoadDll` requires a non-zero remote thread exit code, not just a thread
  that ended.
- `OwnershipHooks` sends Steam a private copy for the masked second dispatch
  (`callbackcopy::ForMaskedGame`) instead of rewriting the caller's buffer in place,
  which Steam could deliver deferred after the real-id payload. `EnsureBufferSize`
  returns bool and verifies the grown range with `utils/memory::IsWritableRange`
  before trusting `put`.
- The payload worker waits on an explicit stop event instead of sleeping, so a detach
  is noticed within 500 ms and no thread sits dormant.

### Steam memory and the wire

- Wire god-files split: `ServiceJobTracker` reads name and id with one
  `ParseFromArray` per frame, `WireDiag` owns eresult names and tracing, and
  `FrameOrigin` captures the connection. Same dispatch and diagnostics, half the
  parsing.
- Packet fields are read with `memcpy` instead of unaligned casts, including the
  DirWatch notify records, which also gained bounds and alignment checks.
- The packet pool has a budget: frames above 256 KiB plus header degrade to
  passthrough with one log line instead of growing for the whole session.
- The packet write barrier is lazy — it runs only when a handler produced a new body
  (`newBodyLen != kNoChange`) — and `PersonaInject` checks for a staged persona
  before paying for it, keeping the packet queued when the barrier refuses.
- The IPC dispatch table is published by an atomic `shared_ptr` swap, so Steam
  threads read lock-free; the snapshot is kept alive while a handler runs, so a late
  re-registration cannot free the map mid-call. The returning pointer in the IPC bus
  is gone.
- PacketRouter evicts the oldest job name instead of growing, and only records
  non-OK eresults. The eticket cache is capped at 1024 entries.
- The dead `GetManifestRequestCode` bridge is deleted (`ManifestBridge`, the
  `ManifestFetch` submit/resolve paths, provider URLs, `bridge_wait_ms`, job hash);
  `[manifest_fetch]` keys left in `aethercore.toml` are logged once and ignored.
- Statistics and VDF handling share one codec: `utils/KeyValues` for binary KV1,
  `VdfText` for text VDF, `ManifestIdentity`, and the IPC-spec parsers were extracted
  and unit-tested, and `SteamStatsCache` was rebuilt on the shared walker.
- `SyncLanguageToSpacewar` no longer uses a regex: it goes through the VDF codec,
  refuses values containing quotes or backslashes, and writes `appmanifest_480.acf`
  atomically.

### Backup, restore and quota

- Backup and restore staging paths are unique per process, and size mismatches report
  the source and destination errors separately — the false backup failures and the
  Desk/DLL collisions on temp files are gone.
- Restore verifies the copied size and counts truncated files as logged errors
  instead of presenting them as success.
- The Lua pin scan is robust: the regex is compiled once and `regex_error` is caught,
  so one bad file is skipped instead of failing the whole backup.
- The daily quota file fails closed: an unreadable file refuses generation with a WARN
  and keeps a `.corrupt-<ts>` copy for diagnosis, instead of silently resetting the
  budget. A missing file still starts at zero.
- The manifest backup index is inverted: one walk per session plus an upsert on every
  write, so lookups are O(1) with no scan per miss and negative memos are invalidated
  immediately.
- The achievement backup worker moved onto the worker registry, every job, checkpoint
  and final copy is exception-isolated and counted, and two `strtoul` end-pointer
  reads off temporary `substr()` strings are fixed.
- A custom pattern mirror no longer blocks anything, but logs one WARN and reports
  itself in the status when configured.

### Threading and shutdown

- `TtlCache::Get` no longer returns through the iterator after unlocking — a
  use-after-invalidate race.
- Worker bodies run inside try/catch with `done` always set, and `StartWorker` takes an
  optional completion future and re-checks shutdown before registering, so a failed
  thread creation leaves no phantom entry.
- The four ad-hoc module threads (license retry, cdkey summary, unlock summary,
  eticket mint) became named workers joined by `workers::Shutdown`.
- Finished workers are collected and joined outside the registry mutex, so
  registration no longer serialises behind a teardown.
- The logger has one write path (`Emit`/`FormatBody`) running through a dedicated
  worker queue with a per-line flush, and the VEH/UEF/terminate handlers drain the
  queue first, so the last lines before a crash survive.
- `PatternEngine` split into `PatternCache` (index I/O) and `PatternResolver`.
- Boot: `InitThreadLogic` extracted into `core/BootSteps` with one function per stage,
  `ManifestFetch` split into `ManifestIndex` and `DepotCachePublisher`, and the hook
  orchestration moved to `core/HookBootstrap`.
- Retry waits are event-driven at the same cadence: the steamui retry wakes on
  `LdrRegisterDllNotification`, the late pattern retry on `FindFirstChangeNotificationW`,
  and the package tick was widened — same budgets, far fewer wakeups.

### Paths, encoding and environment

- A truncated module path is fail-fast: init stops with a clear ERROR before creating
  directories at a wrong prefix, instead of poisoning the session.
- UTF-8 inside, `*W` at the boundary: a shared `utils/WinFile.h` helper (fail-closed),
  a strict `Narrow()`, two-phase registry access and native-path fstreams, so Steam
  paths with non-ASCII characters work.
- The environment block is no longer read for Steam's own processes, is zeroed after
  parsing, and only numeric AppIds leave `ReadSteamEnvApi` into snapshots and logs.
- Tunables moved into `Constants.h` (log, http, inject, watch, eos, staging, quota).

### Security and hardening

- Lua HTTP is HTTPS-only: `RuntimeHttp` Get/Post reject anything else with a 403 mimic
  and a warning, without touching the network. Internal pattern mirrors are
  unaffected.
- Secrets hygiene: ciphertext buffers and DPAPI blobs are zeroed on every path.
- The ABI guard policy (`ApplyTable`, `CountRejection`, counters, `VerdictText`,
  `ReasonText`, the `Global()` resolver) moved inline into headers so tests exercise
  the real logic instead of a stub.
- Closed gaps on callback buffers, reply buffers and child-process injection.
- Guard rails for crash, overflow and stale-process cases across the hook surface,
  plus explicit casts where the compiler was previously trusted to pick.

### Build, quality, tests and docs

- Warning-clean build enforced: `/W4 /permissive-` `/WX` and clang-tidy apply to every
  target, orphan headers fail the build, ctest runs in ALL, and protobuf moved to its
  own `/w` target.
- `AETHER_CLANG_TIDY=REQUIRED` in the `x64-Release` preset: a missing clang-tidy now
  fails configure instead of warning and skipping.
- `clang-format` added with a repo-wide normalisation, and the format gate covers
  `.hpp`, `.inl`, `.c` and `.cc` as well.
- `build_dll.cmd` is the single DLL entry point (format gate, preset build, ctest,
  portable ZIP) and keeps its window open for double-click runs.
- Build is green again: `(void)` on ignored results, references instead of copies,
  checked `localtime_s`/`localtime_r`, overflow-checked digit parsing in `EnvReader`,
  explicit int casts for WinHttp timeouts, and clang-tidy now finds VS2025's LLVM
  path and forwards `/EHsc`.
- Tests split into `core_unit_tests` and a new `wire_contract_tests` binary, with
  `CHECK_EQ` added, plus new unit tests for `callbackcopy`, `memory` and
  `payloadattest`.
- Removed dead weight: the orphan `AetherInjector` (dwmapi proxy) target and
  `common/verify-version.ps1`, a duplicate of `Tools/verify_version.ps1` that nothing
  referenced.
- Samples added under `config/pattern.example/` (steamclient signatures with RVAs,
  steamclientipc method hashes for a Steam beta); nothing reads them at build or
  runtime.
- Docs: `compilazione-veloce.md` became `quick-build.md` with an English build guide,
  the presence design moved into `ARCHITECTURE.md`, dangling references to documents
  that do not exist were dropped, and comments across `AetherCore`, `AetherPayload`,
  tests and `steam_messages.proto` got a full audit (history and "round N" notes
  removed, invariants kept).

### Internal (no behaviour change)

- Presence state, hook bootstrap, pattern engine, boot steps, manifest fetch, packet
  routing and the logger were each split into named units with tests; the ABI guard
  logic moved into headers.
- `GamesPlayed` wire code split into `GamesPlayedFormat`, `GamesPlayedRewriter` and
  `DonorPool`; `OnPersonaStateRecv` split into helpers with its missing diag mutex
  added; `StatusWriter` emits through the new `utils/JsonWriter`.
- `utils/Paths.h` deduplicated `LowerAscii`/`BaseName`/`HasExtension`; `SchemaBuckets`
  moved to a header.
