# Shared contracts — AetherDesk ↔ AetherDLL

Single specification of the **twinned** logic between AetherDesk (Rust/Tauri)
and AetherDLL (C++, in-process in Steam). The two sides run in different
processes, at different times (Desk may be closed; the DLL lives inside Steam)
yet must produce **the same backup tree, the same cache and the same quota
consumption**. This page is the source of truth for the *policy*; the code
implements it twice, the contract tests (§6) protect it.

**Read it when:** you change a value, a rule or a file name that both halves
must agree on, or you add/rename an IPC command.

**Not here:** the internal architecture of one half — see
[`AetherDesk/docs/ARCHITECTURE.md`](../AetherDesk/docs/ARCHITECTURE.md) and
[`AetherDLL/docs/ARCHITECTURE.md`](../AetherDLL/docs/ARCHITECTURE.md).

> **Maintenance rule**: when you change a value or a rule here, you must align
> BOTH implementations and update this table in the same commit. Every section
> lists the two "twins" with precise paths.

## Contents

- [1. On-disk layout (single source of truth)](#1-on-disk-layout-single-source-of-truth)
- [2. Manifest backup/restore contract](#2-manifest-backuprestore-contract)
- [3. "Manifest first, Lua last" commit contract](#3-manifest-first-lua-last-commit-contract)
- [4. Hubcap quota and generation contract](#4-hubcap-quota-and-generation-contract)
- [5. Lua pin contract](#5-lua-pin-contract)
- [6. Contract tests](#6-contract-tests)
- [7. Desk → frontend IPC contract (argument keys)](#7-desk--frontend-ipc-contract-argument-keys)
- [8. The Steam path is not a client parameter](#8-the-steam-path-is-not-a-client-parameter)
- [9. ACF ownership: who writes which field](#9-acf-ownership-who-writes-which-field)
- [10. Log line contract](#10-log-line-contract)
- [11. Client achievement page: local state only](#11-client-achievement-page-local-state-only)

---

## 1. On-disk layout (single source of truth)

| Path | Contents | Writers |
|---|---|---|
| `<Steam>\depotcache\<depot>_<gid>.manifest` | Steam's primary cache | Steam, Desk (resolver/workshop/store), DLL (live generation, restore) |
| `<Steam>\config\depotcache\` | Steam's secondary cache (read-only for us) | Steam |
| `<AetherData>\backup\<app_id>\lua\<app_id>.lua` | **original** Lua (pristine, from the sources) | Desk (`store_original`) |
| `<AetherData>\backup\<app_id>\lua\history\<app>-<millis>[-n].lua` | modified versions, content-deduplicated | Desk (`archive_lua_history`), DLL ("as-missing" copy only) |
| `<AetherData>\backup\<app_id>\lua\<depot>_<gid>.manifest` | referenced manifests (active AND commented pins) + Workshop manifests | Desk and DLL (contract §2) |
| `<AetherData>\state\hubcap_generation_quota.json` (+ `.lock` sibling) | shared daily budget | Desk and DLL (contract §4) |
| `<AetherData>\temp\hubcap\workshop\<item>.manifest` | per-item keyed Workshop cache (Desk only) | Desk |
| `<AetherData>\state\workshop_unavailable.json` | terminal Workshop `404`s, keyed by item + manifest, 24 h lifetime | Desk only |
| `<AetherData>\state\hubcap_game_updates.json` | monitor checkpoint (fingerprint, `contents_checked`, …) | Desk only |

The `<depot>_<gid>.manifest` file name is the manifest's **complete identity**:
two different bytes under the same name do not happen in practice; hence the
copy contract never overwrites (§2).

## 2. Manifest backup/restore contract

Twins:
- Desk: `AetherDesk/src-tauri/src/core/backup.rs` → `backup_referenced_manifests`,
  `backup_manifest_bytes`, `sync_lua_backups_from_stplug_in`.
- DLL: `AetherDLL/AetherCore/hooks/wire/ManifestRestore.cpp` →
  `CopyAtomicallyIfMissing`, `BackupReferencedManifestsAtStartup`,
  `BackupManifestAfterGeneration`, `RestoreLuaDirectory`/`RestoreAppOnce`.

Rules (identical in both processes):
1. only manifests referenced by the current Lua are copied (active or commented
   pins — the regex is not line-anchored, so `--setManifestid` counts as a
   reference); Workshop manifests have a dedicated channel
   (`backup_manifest_bytes` on the Desk side, keyed on the owning app);
2. a **non-empty destination is never overwritten**;
3. a **0-byte destination is a placeholder**: remove and rewrite it;
4. the copy is **atomic** (temp + rename/`AtomicReplace`) and **size-verified**;
5. a failed copy **rechecks the destination**: when the other process completed
   inside the race window, the file counts as present, not as an error.

Restore order (local-first, identical in both):
`depotcache` → `config\depotcache` → `backup\<app_id>\lua`.
When the DLL finds the file in the backup, it **publishes** it to depotcache
before answering hit; Desk (`manifest/resolver.rs`) does the same
restore-then-gate. An empty file is NEVER a local hit.

**Lua** backup: the original is the pristine version (first Lua seen or last
downloaded from the sources); every differing content goes to `history/`,
byte-deduplicated. Archives within the same millisecond take an `-n` suffix
(pin pipelines archive pre+post back-to-back). Anyone rewriting a Lua must
archive **pre AND post** (see §5).

## 3. "Manifest first, Lua last" commit contract

The Lua in `config\stplug-in\` is the DLL hot-reload **trigger**: the moment it
appears, every manifest it references must already be visible in depotcache.
Implementations:
- Desk: `steam/compat.rs` → `install_lua_and_manifest_files` (staging → remove
  old targets → rename manifests → verify → rename Lua → verify).
- Desk: `manifest/pin_refresh.rs` → `refresh_game_pins_from_hubcap` (completeness
  gate BEFORE `realign_commented_pins`).
- DLL: live generation writes to depotcache (temp + `AtomicReplace` + size
  verification) without ever touching the Lua.
- Desk: `application/store/package_completion.rs` → `complete_luatools_package`.
  LuaTools sources that archive manifests answer `/api/manifest/download` with
  `<appid>.zip` (Lua + `.manifest`); sources replicating only the Lua (e.g. Luie)
  answer with a naked Lua whose pins are not on disk. Before
  `install_lua_and_manifest_files`, enabled pins missing from the package and
  not already local are fetched from `GET /api/givemethemanifestpunk/{depot}/{gid}`
  (same session, depot+GID identity check via `manifest::identity`, no daily cap
  consumed); only the remainder goes to Hubcap generation, and only when the
  user has a configured key. Sources are tried in "brings manifests" →
  "Lua only" order (`rank_available_sources`); when the first package cannot be
  completed, ONE fallback source is tried (`LUATOOLS_MAX_SOURCE_ATTEMPTS`): every
  attempt costs one cap download.

## 4. Hubcap quota and generation contract

Twins: `AetherDesk/src-tauri/src/providers/hubcap_generation.rs` ↔
`AetherDLL/AetherCore/network/HubcapQuota.cpp`.

- Daily budget: **1500 game/day**, **500 workshop/day**; reset at fixed **EST
  midnight** (like the provider), not local midnight.
- One state file + `.lock` sibling: every read-modify-write cycle happens under
  lock, so counters stay coherent no matter which process writes.
- **reserve → request → release-on-failure**: a failed attempt returns the
  reserved unit (no transient error burns quota).
- The DLL reserves game generations only (workshop is Desk-only).
- Game generation endpoint: `GET /generate/manifest?depot_id=&manifest_id=`;
  the response must be identity-validated (depot+gid in the protobuf) before
  install.
- Workshop repair targets are direct children of `WorkshopItemsInstalled` with
  a positive decimal manifest. The explicit repair also targets
  `WorkshopItemDetails`-only subscriptions with a positive `manifest`; the
  background monitor never spends Hubcap quota on those subscriptions.
  Per-branch `manifest_history` indices are not items, and an entry without a
  positive `manifest` is not a subscription target even when it has a
  `latest_manifest`.
- Installed items target `latest_manifest` when it is newer than the installed
  manifest (or when timestamps are absent and `NeedsDownload` is set). Repair
  stages the new manifest and reports the update; downloading the payload
  stays a manual Steam action, even when the old content directory still
  exists.
- A terminal Workshop `404` is remembered per `(item, manifest)` for **24 h**;
  a changed manifest retries immediately, and a skipped item does not consume
  the 5-per-pass generation cap.
- The explicit repair processes subscription targets before installed updates.
  The monitor retries installed updates after **90 s**, but it never processes
  subscriptions: a manual run that exceeds the 5-generation cap must be run
  again to drain the remaining subscriptions.

Traffic policy (current values — align here when they change):

| Parameter | Desk | DLL |
|---|---|---|
| Generation serialization | process scheduler, min interval **750 ms** | serialized worker (queue), no long in-pass retries |
| Attempts per generation | 1 (retry lives at lane level) | **2** × **6 s** timeout, 750 ms sleep between attempts |
| Backoff on failure | monitor lane: **30 s × 2^n**, cap **30 min** | proactive: **{30, 60, 120, 300, 600, 900} s**, max **6** retries |
| Bridge window (CM recv) | — | `manifestBridgeWaitMs` default **2500 ms** (clamp 0–10000), then passthrough + background continuation |
| Cap per pass | workshop: **5** remote generations/run (deferred → re-run after **90 s**); pin_refresh: 1 app/poll, **15 min** interval | proactive queue: 1 (gid,depot) key at a time, dedup per key |
| Poll/cadence | monitor: start delay **8 s**, poll **20 s** | DirWatch/event-driven + proactive queue |

Asymmetries are **intentional**: the DLL sits inside Steam's network thread (must
answer in ~ms and cannot queue infinite work), Desk is a UI orchestrator (can
afford periodic lanes and batches). The values must stay compatible though:
e.g. the bridge window (2.5 s) ≥ live-generation p95 (~1 s); the DLL backoff
(max 15 min) ≈ Desk backoff cap (30 min).

## 5. Lua pin contract

- **Active** pin (`setManifestid(depot,"gid")`) = version lock: Steam must stay
  on that gid; automatic pipelines NEVER touch it.
- **Commented** pin = informational: the game is in update mode; the commented
  value must point at the last known version because "Disable updates" turns
  it back on.
- **GIDs are not chronologically ordered**: "newer" does not exist; the signal
  is "different from the pin", prioritizing local evidence
  (`latest_installed_gid`, mtime on depotcache) over the Hubcap view
  (`/manifest/{appid}/contents`, free endpoint).
- Every Lua rewrite (realign, toggle, policy, version, edits) must archive
  pre+post to `history/` and back up the referenced manifests (backup rules in
  §2).

## 6. Contract tests

- **Desk** (`cargo test` in `AetherDesk/src-tauri`):
  `tests/ipc_contract_tests.rs` — §7/§8 rules: registered vs defined commands,
  frontend-invoked names vs registered, argument keys vs Rust signatures, wire
  key shape, no `steam_path` in payloads, unused-command inventory.
  `core/backup.rs::tests` — copy-contract rules 2/3/4 (`backup_manifest_bytes`),
  0-byte placeholder replacement, rejected empty payload, `store_original`
  (Created/Updated/Unchanged), `store_history_version` dedup, same-millisecond
  archive uniqueness.
- **DLL**: no harness in the repo (Windows build). Manual checklist when
  touching `ManifestRestore.cpp`/`ManifestFetch.cpp`:
  1. copy with non-empty destination → unchanged; 2. 0-byte destination →
  replaced; 3. post-copy size verified; 4. Desk/DLL race → no spurious error
  (rule 5); 5. `BackupManifestAfterGeneration` also catches commented pins;
  6. quota: release on failure.
- When a §2–§5 rule changes: update both twins, this page and the tests in the
  same commit.

---

## 7. Desk → frontend IPC contract (argument keys)

The frontend calls commands with `invoke('command_name', { key: value })`.

**Rule**: Tauri 2 derives every argument's key from the Rust parameter name with
`to_lower_camel_case()` and looks it up in the payload with **exactly** that
key (`tauri 2.11.5`, `crates/tauri/src/ipc/command.rs`: `v.get(self.key)`).
There is no snake_case fallback: a wrong key is not a compile error, it is a
Promise rejected at runtime with `command X missing required key Y`.

| Rust parameter | Payload key | Notes |
|---|---|---|
| `app_id: u32` | `appId` | general rule |
| `tail_lines: Option<usize>` | `tailLines` | `Option` → omittable key |
| `game_name: Option<String>` | `gameName` | `trigger_luatools_download` / `prepare_luatools_specific_version_download`: optional hint forwarded to lua.tools (`&game_name=`), labels only the account's download history |
| `include_subscriptions: bool` | `includeSubscriptions` | `sync_hubcap_workshop_manifests`: `true` only from the explicit Repair action; the background monitor calls the Rust function with `false` |
| `show_online: bool` | `showOnline` | §7.1 |

### 7.1 `showonline` is a value, not a key

`aethercore.toml` holds `[presence] default_mode = "showonline"` and the
`showonline_apps` / `aetheronline_apps` / `exclude_apps` arrays. That
**all-lowercase** token is also read by AetherDLL (`AetherCore/core/Settings.cpp`):
shared domain vocabulary, not casing to fix.

The command setting it, though, crosses IPC:

- Rust parameter: `show_online: bool` → wire key `showOnline`
  (`commands/steam.rs::set_presence_default_mode`);
- frontend payload: `{ showOnline: next }`
  (`src/views/settings/SettingsAetherSection.tsx`);
- value written to TOML: the `"showonline"` string
  (`core/presence_config.rs::set_default_mode_in_toml`).

Historically the parameter was named `showonline` (domain token copied into the
parameter name) and the frontend sent `{ showonline }`: it worked by coincidence,
and a "cleanup" rename from only one of the two sides would have silently broken
the presence toggle. Keep the three levels above distinct.

### 7.2 Adding or renaming a command

1. signature in `src-tauri/src/commands/<module>.rs` with `#[tauri::command]`;
2. registration in `generate_handler!` (`src-tauri/src/main.rs`);
3. frontend payload with the lowerCamelCase keys derived from the signature;
4. `cargo test ipc_contract` — `tests/ipc_contract_tests.rs` verifies 1↔2↔3 on
   the real sources and is updated only when the contract changes **on purpose**
   (the `documented_command_contracts_have_the_expected_keys` and
   `DOCUMENTED_UNUSED_COMMANDS` lists).

Style rule: with more than one argument, declare them in a type alias in
`src/types/` (e.g. `PresenceToggleArgs`) instead of inline. **`type`, not
`interface`**: `invoke()` takes `Record<string, unknown>` and an `interface`
has no implicit index signature, so it is not assignable (TS2345).

---

## 8. The Steam path is not a client parameter

**Rule**: no command receives `steam_path` from the frontend. The path resolves
in the backend, from settings, with two helpers in
`src-tauri/src/commands/mod.rs`:

| Helper | When | Behavior without a path |
|---|---|---|
| `command_steam_path(&app)` | the command cannot work without a valid path (install/uninstall, manifest writes, downloads) | `Err` with a user message ("Steam installation path is not configured…") + filesystem validation |
| `configured_steam_path(&app)` | the command must degrade gracefully (DLL version reads, update checks, residue probes) | `None` → the caller answers `N/A` / `0` / "no updates" |

**Allowed exception**: `commands/steam.rs::check_steam_path(path)` validates a
**candidate** path (what the user is typing in Settings, before save). It is
not the configured path, so the client must pass it.

**Why**: with the parameter in the payload every call cost the UI a
`get_settings` (file read + DPAPI keystore decryption) and 22 signatures where
the client could send a path different from the configured one. The rule is
guarded by `tests/ipc_contract_tests.rs::no_command_takes_a_steam_path_from_the_client`.

---

## 9. ACF ownership: who writes which field

`steamapps/appmanifest_<app>.acf` has two writers in Aether, with disjoint
fields:

| Field | Writer | When |
|---|---|---|
| `InstalledDepots.*.manifest`, `InstalledDepots.*.size` | the installed-depot **repair** only (both halves, same rule) | Desk: after the user's install/version/Repair acts on that app. DLL: once per process at Steam startup, managed apps only |
| `buildid`, `TargetBuildID`, `StateFlags`, `BytesToDownload`, `BytesDownloaded`, `UpdateResult`, `SizeOnDisk` | Steam itself | on its own updates |
| `LastOwner` | DLL `core/AppManifestOwner.cpp` (§ family sharing) | after login / boot |
| everything else | Steam | — |

**Rule 0 — a version apply writes only the Lua.** The pins in the `.lua` are
the single statement of which build the user wants; the ACF describes what
Steam installed. Writing `buildid`/`TargetBuildID` from a pick claims the
client already runs a build it never downloaded, so Steam keeps the card on
PLAY until the game is launched and closed. Aether leaves both fields to Steam.

**Rule 1 — a version apply never writes `InstalledDepots`.** The pinned GID a
version selects is the target of the next download, not the identity of the
content already on disk. Writing it into `InstalledDepots` makes Steam ask the
CM for a manifest the account cannot fetch.

**Rule 2 — the repair only restores a GID that exists locally.** For an entry
whose declared GID has no non-empty `<depot>_<gid>.manifest` in `depotcache` or
`config\depotcache`, and only for apps Aether manages (the Lua declares the
depot):
1. pick the local file whose own content size (`ManifestContentSize`) equals the
   ACF's recorded `size` — that is the file the installed content came from;
2. otherwise pick the newest local file (write time, GID breaks ties);
3. rewrite `manifest` (and `size`, when it is known) in place, atomically,
   preserving a read-only attribute. Every other byte of the ACF stays as is.

No local manifest for that depot → no rewrite. Declared GID `0` → nothing to
repair. An unwritable ACF is logged and left for the next attempt; neither half
retries in a loop.

**Rule 3 — no background polling.** The DLL does one pass at boot (before Steam
builds this session's app list) and never again in that process; the Desk does
it only inside the action that triggered it.

**Rule 4 — a pin change reaches Steam as a recheck, never as an ACF write.**
The pins are the target; the ACF is what Steam installed. A version apply ends
when the Lua is written (Rule 0): the Desk must not nudge the client by touching
`buildid` or `TargetBuildID`. Instead, after a Lua reload the DLL diffs the
pinned `(depot, gid)` rows of each changed file and asks the client to rebuild
only the apps whose pins moved — `MarkAppChange` on the UI thread, from a queue
of 64, at most 8 per frame — and Steam decides PLAY, UPDATE or a download from
the pins against `InstalledDepots`. A missing `MarkAppChange` /
`CSteamUIAppControllerRunFrame` pattern disables the recheck alone
(`recheck_state = unsupported` in `status.json`); card repair, ownership and
membership keep working.

**Rule 5 — the owner banner is display-only.** `[library] owner_banner_apps`
writes `LastOwner = owner_banner_steamid` for apps Aether spoofs, and
`family/FamilyLenders.cpp` reads any foreign `LastOwner` as a real Steam Family
lender. The classification must keep those two apart: the app is drawn as
shared (`luadata::IsFamilyShared`, `AppOwnership.steamId32`, the overview flag)
but it is still one Aether provides. `core/FamilyClassification.h` is the single
rule — a `RealShare` removes the app from `HasDepot`, a `Banner` does not — and
`luadata::IsRealFamilyShare` is the behavioural read every gate uses (depot
keys, manifest overrides, the cloud gate's `family-shared`, the achievement
page). Without it the stamp switches off every `HasDepot` consumer for the
listed apps, achievements included.

Twins:
- Desk: `steam/acf.rs` → `repair_installed_depots`,
  `repair_installed_depots_for_app`; size semantics in
  `manifest/identity.rs` → `manifest_content_size`. A version apply has no ACF
  writer; the reading half is the private `read_build_fields`, pinned by the
  tests in `steam/acf.rs` (Rule 0).
- DLL: `core/InstalledDepotRepair.{h,cpp}` → `PlanInstalledDepotRepairs`,
  `ApplyInstalledDepotRepairs` (boot step 10d in `core/BootSteps.cpp`); size
  semantics in `network/ManifestIdentity.cpp` → `ManifestContentSize`. The
  recheck side is `scripting/DirWatch.cpp` → `RequestAppRecheck` (Rule 4).

Decision matrix under test: the Desk cases in `steam/acf.rs::tests`, the DLL
`installeddepots`, `manifest_identity` and `recheck` ctest cases.

**Why**: `InstalledDepots` is the record Steam computes every download delta
from (`contentupdatecontext.cpp` / `depotdownloadmgr.cpp`). A GID with no local
manifest is denied by the CM, the download aborts with `Access Denied` and Steam
tells the user "No connection", even though the content is on disk and another
local manifest describes it. Re-downloading the Lua cannot fix it: the Lua
changes the target, not the installed record. Nothing repairs it either while
desk and Steam disagree about the same file, so the repair runs where the ACF is
read: at Steam startup.

---

## 10. Log line contract

`desk.log` and `main.log` are read as one document: the Logs view merges them
with each other and with `uc_online2.log`, and `export_logs_bundle` ships them
together as a support artifact. One shape, both halves:

```
[YYYY-MM-DD HH:MM:SS.mmm] [PID:12345] [TID:1234] [INFO ] [Module    ] message
```

- **Absolute local date, always.** It is the merge key: `commands/logs.rs`
  orders merged lines on the first bracket (`sort_key`, which also feeds the
  line's position in the export), so a bare `HH:MM:SS` cannot be ordered across
  midnight and cannot be placed next to Windows' own logs. Fixed width:
  millisecond precision, zero padded.
- **`PID` is Desk-only.** `main.log` is written by Steam and by every game
  process under one cross-process mutex, so a PID there would add bytes without
  identifying anything the `TID` does not.
- **Level tags are fixed width** (`TRACE`, `DEBUG`, `INFO `, `WARN `, `ERROR`),
  because the view derives the line class from the bracketed tag.
- **Module tag** names the concern (`store`, `library-watch`, `Depot`,
  `License.Hooks`); Desk capitalizes the first letter, the DLL does not. The
  merged view prepends its own source tag instead: `[DESK]`, `[DLL ]`, `[UCO2]`.
- **No ANSI, no multi-line messages.** A stack or a payload is one line.

Twins:
- Desk: `core/logger.rs` → `format_line` (shape) and `format_timestamp_ms`
  (date); merged ordering and source tags in `commands/logs.rs` →
  `merge_tagged`, `sort_key`.
- DLL: `core/Logger.cpp` → `FormatTimeAndThread` (date, buffer 64) and `Emit` /
  `EmitSync` (shape).

Decision under test: `core/logger.rs::tests::formatted_line_carries_an_absolute_date_and_the_shared_shape`
and the `wire_logger_line_carries_the_date` ctest case assert the same shape
from both halves; `commands/logs.rs::tests::merged_logs_sort_across_days_and_sources`
covers the ordering the date exists for.

### 10.1 Export bundle layout (Desk only)

The `Save` button in the Logs view ships
`Downloads\AetherLogs_HH-MM-SS.zip` (built by
`AetherDesk/src-tauri/src/commands/logs.rs::export_logs_bundle_sync`) with
one folder per artifact family:

```
AetherLog/  desk.log.txt, desk.log.last.txt, desk_status.json.txt
            aetherdll_main.log.txt, aetherdll_main.log.last.txt, aetherdll_status.json.txt
            uc_online2.log.txt (normalized to [UCO2]), donors.txt (no time suffix)
SteamLog/   <original Steam names, already filtered>
```

- `AetherLog/` keeps a time suffix (`desk.log_14-32-05.txt`) because each
  export is its own snapshot; files carry the millisecond stamp the §10
  contract guarantees.
- `SteamLog/` collects only `<Steam>\logs\` files named in
  `AetherDesk/src-tauri/src/core/steam_logs.rs::SELECTED_STEAM_LOGS`, plus
  Steam's own `.previous.txt` session twin of each, and only when the file's
  mtime is within the last 12 hours. Lines are kept when their
  `[YYYY-MM-DD HH:MM:SS]` Steam stamp is within the window; a file without
  any stamp line is included whole, tail-capped at 4 MiB. Steam stamps are
  second-precision, so Desk's millisecond stamps are what order the two
  families against each other.

---

## 11. Client achievement page: local state only

Steam's own achievement page (library → game page → Achievements) is served by
two service methods the client sends together: `Player.GetGameAchievements#1`
(the schema: names, icons, `internal_key`, `schema_hash`) and
`Player.GetUserAchievements#1` (the account state, keyed by `internal_key`).
The CM answers the schema for any app but sends an empty state body for an app
the account does not own, so every app Aether advertises shows an empty page
while the overlay has the real unlocks.

**Rule — the page lists every achievement, from the local cache, never a guess.**
Aether rebuilds the user-state reply for a managed app (`luadata::HasDepot`) and
leaves every other app alone:

| Decision | Owners | Result |
|---|---|---|
| not managed (`not-managed`) | DLL `hooks/wire/AchievementUi.cpp` | Valve's reply, byte for byte |
| schema or local state not ready yet (`no-schema`, `no-state`) | DLL, same file | the reply is held: the client gets a copy aimed at a job nobody waits on (so its real job stays pending) and the rebuilt frame is delivered when the data lands. After 8 s the original is released |
| data ready (`rebuilt`) | DLL, same file | one entry per achievement in the captured schema, locked ones included, `progress_float` always, unlock pair only when unlocked; one `is_achievable` group per schema group; `schema_version`/`schema_hash` and unknown fields kept from Valve's reply |
| reply too large, unparsable, header unparsable | DLL, same file | Valve's reply (fail open) |

- **Twins.** The decision table is `hooks/wire/AchievementUiPolicy.h` (pure, no
  logger, no `g_state`, no Windows; `FillReply` writes the body as a template so
  no protobuf type is declared there) and its consumer is
  `hooks/wire/AchievementUi.cpp`. Change one, change the other.
- **Sources.** Unlocks and dates come from
  `appcache\stats\UserGameStats_<account>_<appid>.bin`, read on a
  `workers::Submit` job; the schema comes from the `GetGameAchievements` reply
  the client already received. Nothing is uploaded and no unlock is invented.
- **Body shape.** One entry per schema achievement is what makes the page appear
  for an app whose account has no unlocks at all: Steam draws the rows from this
  reply, not from the schema, and an entry without `progress_float` leaves the
  row blank. This is the reference behaviour.
- **eresult.** The CM marks an unowned app denied; the reply is rewritten to
  `k_EResultOK` (the app is not denied, it simply has no CAuth). The header is
  rebuilt in the router's scratch buffer, body untouched on that path.
- **Holding.** A reply that arrives before the schema or before the `.bin` read
  finishes is parked with its real header/body, and the client is handed a copy
  whose `jobid_target` is `1` (a job the client manager never allocates, so the
  copy is dropped and the real job stays open). The schema reply and the worker
  queue the rebuilt frame; the `RecvPkt` hook delivers it on the next inbound
  carrier (`AchievementUi::TryDeliver`, the same trick as `PersonaInject`).
- **Keys.** `internal_key == bucket * 256 + bit` (measured on the beta client). Entries keep the schema order and only keys
  present in the captured schema are emitted; key `0` is a real key, so
  presence of the field decides, never its value.
- **Unconfirmed bits.** A bit reaches the reply when either the bucket's `data`
  mask or its `pendingbits` mask holds it: for an app the account does not own
  the server never acks the bit, so it only ever appears in `pendingbits`.
- **The schema reply is never rewritten**, and `schema_hash` is never derived
  (it cannot be computed from the `.bin`): the rebuilt body re-emits Valve's.
- **Tests.** `AetherDLL/tests/core_unit/achievement_state.cpp` (local reader),
  `achievement_policy.cpp` (decision table: zero unlocks, order, key 0, hold)
  and `achievement_service.cpp` (golden blobs captured from the client, which
  pin every field number and the rebuilt body shape).
- **Switch.** `[achievements] ui_service` in `aethercore.toml`, default `true`.
  Off means the reply is Valve's; nothing else changes.

---

## Related

- [`AetherDesk/docs/ARCHITECTURE.md`](../AetherDesk/docs/ARCHITECTURE.md) —
  Desk module map, mutation ownership and settings repository.
- [`AetherDLL/docs/ARCHITECTURE.md`](../AetherDLL/docs/ARCHITECTURE.md) —
  AetherCore invariants.
- [`docs/glossary.md`](glossary.md) — shared vocabulary.
- [`docs/quality-gates.md`](quality-gates.md) — the gates that run the
  contract tests.
