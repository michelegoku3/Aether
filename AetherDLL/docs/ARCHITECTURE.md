# AetherCore Architecture Notes

**What it is:** the invariants that keep AetherCore simple and maintainable.

**Read it when:** you add state, a hook, a worker, a persistence path or a
subsystem to the DLL.

**Not here:** the rules both halves must honor
([`docs/shared_contracts.md`](../../docs/shared_contracts.md)), the Desk
architecture ([`AetherDesk/docs/ARCHITECTURE.md`](../../AetherDesk/docs/ARCHITECTURE.md)),
the fast build loop ([`quick-build.md`](quick-build.md)).

## Contents

- [Core rule: shared runtime state lives in `AetherCoreState`](#core-rule-shared-runtime-state-lives-in-aethercorestate)
- [Allowed module-local exceptions](#allowed-module-local-exceptions)
- [Disallowed patterns](#disallowed-patterns)
- [Naming guidance](#naming-guidance)
- [Review checklist for new state](#review-checklist-for-new-state)
- [Achievement diagnostics (logging + backup)](#achievement-diagnostics-logging--backup)
- [Runtime publication and lifecycle](#runtime-publication-and-lifecycle)
- [Event-driven persistence, workers and hot paths](#event-driven-persistence-workers-and-hot-paths)
- [Manifest acquisition, presence session and module splits](#manifest-acquisition-presence-session-and-module-splits)
- [Backup lifecycle and pattern boundaries](#backup-lifecycle-and-pattern-boundaries)
- [Pending version changes](#pending-version-changes)
- [Library membership](#library-membership)
- [SteamUI app overview](#steamui-app-overview)
- [App recheck after a pin change](#app-recheck-after-a-pin-change)
- [Presence channels (`-showonline` / `-aetheronline`)](#presence-channels--showonline---aetheronline)


## Core rule: shared runtime state lives in `AetherCoreState`

Any mutable data that is:

- read or written by more than one subsystem;
- read or written by more than one thread;
- needed for diagnostics/status reporting;
- part of the domain model of AetherCore;
- or long-lived beyond one function call

should live in `AetherCoreState` or behind a small API that stores its shared data there.

Examples already centralized:

- Lua-provided depots, tokens, manifests, pending hot-reload changes;
- manifest and e-ticket runtime caches;
- pipe snapshots;
- online payload injection counters and PID set;
- pattern indexes and IPC spec hashes;
- cloud gate log-dedup state;
- game-name resolver captured object and name cache;
- presence runtime state;
- pending encrypted-ticket calls;
- hook manager registry.

## Allowed module-local exceptions

Module-local state is allowed only when it is not AetherCore domain state and one of the following applies.

### 1. Hook trampolines and init-time hook plumbing

Original function pointers such as `o_CheckAppOwnership`, `o_RecvPkt`, or `o_GetAppDataFromAppInfo` are part of hook wiring. They are written during hook installation and then treated as immutable call targets.

They should stay beside their hook bodies because moving them to central state would make the code harder to read without improving safety.

### 2. Private lifecycle of a single service module

A module may own its private control block when the data is only used to run that module and not consumed by the rest of the program.

Examples:

- `ScriptEngine` owns the `lua_State` and the mutex that serializes interpreter access;
- `DirWatch` owns its watcher thread/control block;
- `Logger` owns its file handle, mutexes and diagnostic ring.

Important: any domain data produced by these services still goes through `AetherCoreState`. For example `DirWatch` may own its thread, but Lua file contributions are stored through `LuaData` into `g_state.lua`.

### 3. Immutable dispatch tables populated before hooks are armed

Tables that are built once on the init thread and then read-only after hook installation may remain module-local.

Example: `IPCBus` handler table. It is code-dispatch metadata, not changing runtime user state.

### 4. Per-thread scratch buffers

Temporary buffers used only by one thread may be `thread_local`.

Example: `PacketRouter` uses per-thread scratch buffers and a per-thread frame pool. This avoids global shared mutable state in the hook fast path.

### 5. Separate DLL micro-state

The payload and injector DLLs do not have access to the main `AetherCoreState` lifetime. Small module-local state is acceptable there, but it must remain minimal and private.

## Disallowed patterns

Do not add:

- new mutable `g_*` containers for runtime/domain data;
- singleton managers for shared state unless they are pure services with private lifecycle;
- unsynchronized shared buffers in hook paths;
- duplicate caches for the same domain data in multiple modules;
- fallback state hidden inside feature modules.

If a new feature needs shared data, add a small sub-struct to `AetherCoreState` with its own lock/atomic fields and document ownership.

## Naming guidance

- Use `g_state` only for the single central state instance.
- Prefer `s_*` for private module-local service state.
- Prefer `t_*` for `thread_local` scratch state.
- Keep `o_*` for original hook trampolines.

This makes grep-based audits useful: a new mutable `g_*` should be treated as suspicious unless it is `g_state` or a clearly documented legacy exception.

## Review checklist for new state

Before adding state, answer:

1. Is it shared across modules or threads?
2. Is it runtime/domain data rather than service plumbing?
3. Does it need to appear in `status.json` or diagnostics?
4. Does it need locking or atomics?
5. Can it be local, `thread_local`, or derived instead of stored?
6. If it remains module-local, which allowed exception above justifies it?

Default decision:

- shared domain state -> `AetherCoreState`;
- private service lifecycle -> module-local `s_*` with comment;
- hook trampoline -> module-local `o_*`;
- per-thread scratch -> `thread_local t_*`;
- temporary function data -> local variable.

## Achievement diagnostics (logging + backup)

`hooks/wire/AchievementModule.cpp` (wire logic + logging, backed by the
`hooks/wire/AchievementBackup` persistence module) instruments every stage of the user-stats
flow with `[eMsg ...]`-prefixed log lines, so a single `main.log` session shows
the whole story: request spoofing (151/818), donor response rewriting
(152/819, including the data stripped from the donor and the risky
"response not correlated to our spoof" path, logged at WARN), and the in-game
unlock commits (5466) with `*** ACHIEVEMENT UNLOCKED ***` lines.

Because Steam never acks `StoreUserStats2` for apps the account does not own,
the client's local cache (`appcache\stats\UserGameStats_<account>_<appid>.bin`)
is the ONLY copy of the user's progress (`PendingChanges > 0` in that file is
the tell-tale). Every unlock is therefore also mirrored into the AetherDesk
backup tree (resolved via `aethercore\desk_path.cfg`, with an `aethercore\backup`
fallback), using Steam's own per-(account, app) naming:

```
<AetherData>\backup\<appid>\achievements\
    UserGameStats_<account>_<appid>.json    snapshot of unlocks (mirror format)
    UserGameStats_<account>_<appid>.bin     copy of Steam's cache file
    UserGameStatsSchema_<appid>.bin         copy of the game schema
```

All disk I/O runs on the lazy-started `achievement_backup` worker owned by
`core/Workers` (`RecordUnlock()` only enqueues — no filesystem work on
Steam's network thread). `FlushOnShutdown()` is BLOCKING: it drains the queue,
copies the `.bin` files and waits for worker completion (the registry owns the join). It is only called in the explicit
off-loader-lock shutdown path, NOT automatically on normal Steam termination.
Even when called, it cannot guarantee Steam has flushed its cache. The JSON is rewritten atomically on each
unlock (merge keeps the earliest unlock time). If the Steam cache is ever
wiped again, restore by copying the `.bin` back into `appcache\stats` (Steam
closed) or rebuild it from the JSON with `Tools/achievement_decoder.py rebuild`.

`Tools/achievement_decoder.py` decodes/encodes the Steam cache, schema and
mirror JSON files (`decode`, `snapshot`, `rebuild` commands) — use it together
with `main.log` when investigating achievement issues.

### Client achievement page (service path)

The library's Achievements tab does not use the 151/818 path: the client asks
the CM with `Player.GetGameAchievements#1` (schema) and
`Player.GetUserAchievements#1` (account state). The server answers the schema
for any app and an empty state body for an app the account does not own.

- **Modules.** `hooks/wire/AchievementUi.cpp` is the wire glue (send/receive,
  job correlation, schema/state cache, held replies, logging);
  `hooks/wire/AchievementUiPolicy.h` is the pure decision table and body model;
  `hooks/wire/AchievementUserState.cpp` parses
  `UserGameStats_<account>_<appid>.bin` into `internal_key -> unlock time`.
  State lives in `AetherCoreState::AchievementStore::ui`.
- **Body shape.** One entry per schema achievement,
  locked ones included, `progress_float` always present, the unlock pair only
  when unlocked, and one `is_achievable` group per group the schema reply lists.
  The page builds its rows from this reply, so a reply with only the unlocked
  entries (or none) leaves the section blank for an app with no unlocks.
- **eresult.** Valve's header is rewritten to `k_EResultOK`: the CM marks an
  unowned app denied, but the page is not denied, it just has no data.
- **Cold data is held, not skipped.** If the schema or the `.bin` read is not
  ready when the reply lands, the real header/body are parked and the client is
  handed a copy whose `jobid_target` is `1` (never allocated by the job manager),
  so its job stays open. `OnRecvSchema` and the worker queue the rebuilt frame;
  `AchievementUi::TryDeliver` (called at the top of the RecvPkt hook, before the
  packet is dispatched) delivers it on the next inbound carrier. An 8 s timeout
  releases Valve's reply unchanged.
- **Fail open.** An app that is not managed passes through; unparsable bodies or
  headers and a rebuilt body over `kWireMaxBodyBytes` also leave Valve's frame
  untouched, with a `lastReason` for `status.json` (`not-managed`, `no-schema`,
  `no-state`, `too-large`, `reply-unparsed`, `header-unparsed`, `off`).
- **Worker boundary.** The `.bin` read and KV1 walk run on `workers::Submit`;
  the network path only copies cached values under one short mutex. The account
  comes from the request's `steamid`, never from the registry or
  `loginusers.vdf`: that would be disk I/O on a Steam network thread. A request
  that names no account answers "nothing unlocked" instead of waiting.
- **Never fabricated.** Only keys present in the captured schema are emitted;
  the hash is never derived (it is not computable from the `.bin`) and Valve's
  `schema_version`/`schema_hash` are re-emitted untouched.
- **Diagnostics.** `achievement_service_calls`, `achievement_schemas`,
  `achievement_rebuilt`, `achievement_passthrough`, `achievement_held`,
  `achievement_delivered`, `achievement_held_now`, `achievement_last_reason`;
  a `DEBUG` line per capture, hold and rebuild names the app and the counts.

### Donor rotation and schema publication

The donor pool is walked to the end in ONE session instead of one slot per
session. `hooks/wire/DonorProbe` composes the requests the Steam client does not
send: a donor that answers `WITHOUT data` starts a walk, and the walk asks every
slot the app has not tried yet (`DonorPool::NextUntried`) over the service path
(151 with `Player.GetUserStats#1` and a job id of its own; the attempt carries
`probe = true`, which is what routes the response back instead of forwarding
it). The first donor carrying a schema pins itself as the app's preferred
donor, ends the walk, and `hooks/wire/SchemaCache` publishes the blob as
`appcache\stats\UserGameStatsSchema_<appid>.bin` (byte-identical to the
response's `schema` field, and only when its root key is that app id) plus the
AetherData backup copy; the client then serves the game from that file,
including after a restart. An existing cache file is never replaced and a blob
without an achievement bucket is never written: a real Steam download always
wins.

Learning survives restarts in `<steam>\aethercore\donors.txt` (per app:
preferred donor, tried mask, exhaustion stamp). A pool nobody answered from is
left alone for seven days (`donorprobe::kRetryWindowSeconds`), so an app no
donor owns costs one walk per week instead of a full walk per session. Probe
responses are consumed internally: the body is emptied before the frame
continues, so donor progress never reaches the game.

The pool is compiled in: `AetherCore/hooks/wire/DonorIds.h` holds the 15 LumaCore
SteamIDs plus the donors discovered since (today the one that owns The Loopler).
Adding a donor is one line in that header and a release, so every install ships
the same list and there is no per-user file to get wrong. The list is
**append-only**: per-app learning stores donor indices, so an entry is added at
the end and never inserted or reordered. To find a donor for a game nobody owns,
append candidates, let the walk try them, and read
`Donor probe: AppID <app> schema found on donor <steamid> (slot <n>)` from the
log; then keep only that entry. The learning snapshot carries the pool
fingerprint, so a release that changes the list discards stale per-app learning
instead of skipping the new donor as "already tried". Probe requests are paced
(`kMinProbeGap`) through the worker queue, never slept on the recv thread, and a
`LimitExceeded`/`RateLimitExceeded` answer stops every walk for the session.


## Runtime publication and lifecycle

* The DLL attaches to the live `steamclient64.dll` only. There is no copy of the
  client and no `steamui` redirect: every hook must run on the module the running
  Steam instance calls. A second mapping of that name is rejected at attach time
  rather than hooked, because a hook on a module Steam never calls would look
  installed and do nothing.
* The client is never loaded by the DLL. `LoadLibrary` on it from the init thread
  initializes it out of Steam's boot order and crashes `steam.exe` inside the C++
  exception machinery (an access violation in `VCRUNTIME140.dll` before the login
  screen). The attach therefore waits for Steam's own mapping, up to
  `kSteamclientAttachTimeoutMs`, and hooks are installed only once the handle
  exists. The budget is short because the wait runs before the safety nets (the
  achievement backup must precede the client's login reconcile); a timeout logs
  `steamclient_attach = "live-not-mapped"`, defers the steamclient batch, and
  leaves the late-pattern retry (10 s cadence) re-probing the handle.
* Settings are immutable `shared_ptr<const Settings>` snapshots, atomically
  published. Each reader retains its owner for the duration of access. Only
  DirWatch polls the TOML at runtime (about 1 s, including without Lua watches).
  Invalid edits retain the last good snapshot. Lua watch-directory changes need
  a restart; changing settings does not rebuild every initialized service.
* HookManager serializes registry operations; `Snapshot()` returns owned copies.
  Bootstrap serializes whole batches. Status requests only increment an atomic
  counter; one writer coalesces them at 100 ms intervals. Metadata uses
  `statusMetadataMutex`; runtime counters keep their existing locks/atomics.
* IPC maps are published ONCE before setting atomic `loaded`; no reader may
  touch the maps before acquiring that flag. Init attempts are serialized.
* Pattern indexes are built locally on late retry, then published under
  `patterns.mutex`; lookups/availability checks use a shared lock.
* DllMain detach only sets the shutdown flag: no joins, logging, mutex-taking
  flush, hook teardown or persistence. Logger already flushes each written line.
  `AetherCoreShutdown` is an explicit terminal shutdown API for a host that has
  quiesced activity, called off loader lock; no production caller uses it.
  PinSelf prevents normal FreeLibrary unloading. It is not a worker
  registry/quiescence protocol and does not make arbitrary live unloading safe.

## Event-driven persistence, workers and hot paths

* Achievement persistence is EVENT-DRIVEN, not shutdown-driven: the backup
  worker runs a periodic checkpoint (5 min: forced .bin re-copy of every
  touched (app, account) + playtime refresh) and `SessionEnded()` schedules a
  delayed final copy (~15 s) when a game session ends. Worst-case loss on a
  hard exit is one checkpoint interval; `FlushOnShutdown()` remains for the
  explicit shutdown path and also drains delayed jobs immediately.
* Background work lives in `core/Workers`: one task queue (one-shot jobs,
  drained and joined at explicit shutdown) plus a named worker registry with
  stop flags. There are NO `std::thread(...).detach()` calls in
  AetherCore. Workers stop only in the explicit off-loader-lock shutdown;
  DllMain detach still only sets the global shutdown flag.
* PipeWatch handshake inspection (OpenProcess, remote env read, image query,
  payload injection) runs on the task queue, not on Steam's IPC thread; only
  the pid read and the enqueue stay synchronous. Value types only cross the
  thread boundary — never Steam pipe pointers. `AppIdForPipe` reads the appId
  under the snapshots lock without copying the snapshot.
* `FindLocalManifest` uses a local manifest index: positive entries are
  re-verified with one stat per hit; negative entries suppress the backup-dir
  walk for 30 s (matches the existing proactive backoff, so newly generated or
  restored manifests are still discovered on the next window).

## Manifest acquisition, presence session and module splits

* Manifest acquisition is local-index + authenticated Hubcap generation. The
  request-code bridge (`hooks/wire/ManifestBridge`) only earns Steam a
  `manifest_request_code = 0`: it answers a correlated job and only when the
  exact manifest is already in depotcache or the AetherData backup, on Steam's
  network thread, without waiting and without rewriting the request.
  `[manifest_fetch]` TOML keys are deprecated: Settings logs a one-shot WARN
  and ignores them.
* Depot plan: Steam builds depot rows only for the depots its AppInfo lists, so
  a `.lua` app without AppInfo plans just the shared/dependency rows. Aether
  does not write into Steam's depot table: it does not own that vector's type
  or its allocator, and growing it through the package-0 helper corrupts it.
  What the plan contains is Steam's answer plus the manifest overrides below.
* Installed-depot repair: `core/InstalledDepotRepair` runs once per process at
  boot (BootSteps step 10d, on a worker thread, managed apps only). For an ACF
  entry whose GID has no non-empty local manifest it restores a file that does
  exist — same content size first, newest otherwise — because Steam computes
  every download delta from that GID: an unfetchable one aborts with
  `Access Denied`, which Steam reports as "No connection". It writes only
  `InstalledDepots`, atomically, and nothing polls afterwards
  ([`docs/shared_contracts.md` §9](../../docs/shared_contracts.md#9-acf-ownership-who-writes-which-field)).
* App info: `hooks/wire/ProductInfoRequest` asks the CM for one app's PICS
  record when a `.lua` is published or a masked friend needs its icon. The
  queue is one-shot per app per process, and the per-app request is what fills
  the cache when the batch PICS job sits in `Timeout` for the whole session.
* Owner records: `core/AppManifestOwner` never writes an appmanifest while
  `AppState.StateFlags` says an update is running, and never rewrites a plan it
  already settled in this session. A manifest Steam is rewriting is Steam's.
* Presence state (real app behind the 480 mask, showonline, UCO2/OFME spoof
  flag) lives in `hooks/aetheronline/PresenceSession` — owned by
  AetherOnlineHooks::h_SpawnProcess through `Publish()` (single atomic
  publication of all fields), read by everyone via immutable snapshot or
  hot-path atomic accessors. `EndSession()` resets the state when Steam
  reports zero running games, closing the stale window between game exit
  and the next SpawnProcess.
* Module boundaries: `GamesPlayedFormat` (pure wire-format helpers, unit
  tested), `GamesPlayedRewriter` (the two presence rewrite passes as one
  deterministic function), `DonorPool` (donor learning + send/recv
  correlation for the UserStats spoof), `DonorProbe` (the donor walk state
  machine, wired to the wire layer and the schema cache by
  `AchievementModule::InstallDonorProbe`) and `SchemaCache` (validated
  publication of a probed schema into Steam's stats cache). AchievementModule
  keeps a single `SerializeTo` tail instead of eight duplicated blocks.

### Backup lifecycle and pattern boundaries

- `AchievementBackup` keeps queue scheduling and persistence ordering. The
  named worker registry owns its thread. Either `FlushOnShutdown` or the
  registry stop flag seals the queue and makes **all** delayed jobs due.
- The worker observes registry stop within a 250 ms idle polling interval;
  this is not a deadline on disk I/O. Final copies remain blocking. Flush
  waits outside the queue mutex on a shared completion signal and is safe
  before or after registry shutdown. Task exceptions are logged and the
  remaining queue is still drained; completion does not imply all I/O succeeded.
- `PatternEngine` owns initialization, concurrent module loading and index
  publication; `PatternCache` owns TOML/cache/provenance/download policy;
  `PatternResolver` owns lookup, bounds/signature verification and the existing
  ABI sentinel call. Public entry points and safety gates are unchanged.

### Pending version changes

A pin change states intent; Steam derives the state. The pins in the `.lua`
record the build the user picked, and the ACF records what Steam installed:
`buildid`, `TargetBuildID` and `StateFlags` stay Steam's own fields
([`docs/shared_contracts.md` §9](../../docs/shared_contracts.md#9-acf-ownership-who-writes-which-field)).
Aether announces a selection by writing the Lua and nothing else, so a card that
still shows PLAY at the old build is the client's truthful answer until it looks
again.

The client's depot plan comes from the pins AetherCore reads when it loads the
`.lua`, so an app whose pins moved keeps the plan the client built at startup
until something asks it to look again. The DLL closes that gap after a Lua
reload. `scripting/DirWatch.cpp::ApplyChanges` snapshots the pinned
`(depot, gid)` rows of each changed file before and after the reload — a
parse, or an unload that drops every pin the file carried — unions the two
snapshots by app id, and hands the pairs to `SelectRecheck`: only the apps whose
pins really moved survive, because both sides are normalized first (a reordered
reload is not a change) and the result is sorted and deduplicated. A reload that
touched no pin queues nothing and refreshes no card. The survivors go to
`RequestAppRecheck`, which only fills the bounded queue that
[App recheck after a pin change](#app-recheck-after-a-pin-change) drains on the
client's UI thread through `MarkAppChange`; the frame budget and the
`status.json` keys live there. The thread that wrote the Lua never touches a
Steam object. The recheck starts no download by itself; Steam compares the
pinned GIDs with `InstalledDepots` and decides PLAY, UPDATE or a download by its
own rules.

A missing `MarkAppChange` or `CSteamUIAppControllerRunFrame` pattern disables
the recheck and nothing else. Card repair, ownership and membership keep
working, the reload path skips its snapshots entirely (`AppRecheckSupported`),
and `status.json` reports the missing pair rather than a queue that never
drains. A client that never calls `MarkAppChange` leaves the ids waiting rather
than inventing an owner for a call the DLL makes.

## Library membership

A `.lua` app becomes visible only through the **license path**: package 0 carries
every configured id (`LicenseManager::SeedPackage0`), and
`MarkLicenseAsChanged` + `ProcessPendingLicenseUpdates` make Steam re-read it.
Steam then asks for the app's AppInfo and builds the card, the depot plan and
the launch config from that. Nothing else may add an app to the library:
returning ids from the client's subscribed-app list creates an overview entry
before the AppInfo/release-state bookkeeping Steam performs for licensed apps,
which the 11.18 client asserts on and which renders a card with no size and no
launch config.

`luadata::LibraryAppIds()` is the numeric-filename root population for per-app
PICS requests and name lookups (`ScriptEngine`, `DirWatch`, `GameNameResolver`,
`InstalledDepotRepair`); it is not membership and may be empty for a `.lua` with
a non-numeric name, which still enters the library through its seeded ids.

A genuinely owned app gets a package-0 row like every other configured id, but
its ownership answer passes through untouched: the spoof acts only when Steam
did not provide the app (`luadata::HasDepot`), and the family/owned flags keep
the lender banner. `status.json` reports the populations separately
(`membership_owned`, `membership_spoofed`, `membership_family_shared`,
`membership_library_roots`, `package0_app_ids`) so a support log can tell
"not seeded" from "seeded but not classified".

## SteamUI app overview

AetherCore hooks `steamui.dll` only for the app-overview layer. The module is
waited for like the client and never loaded, so the hooks run on the image Steam
maps inside `steam.exe`. `BootSteps` probes it without a budget and the
`steamui_attach` worker waits up to 30 s; a build that maps it later leaves the
layer off for the session and `status.json` says so.

The only object the layer touches is the `CSteamApp` prefix declared in
`core/SteamTypes.h`: `gameId`, `appId`, `releaseState`, `ownershipFlags` and
`appStateFlags`, ending at +0x20. Nothing after that offset is declared, because
its alignment depends on members Aether never reads; a full struct would guess
the tail and put a write in the wrong field on the first layout change.

`FillInAppOverview` is a pass-through detour: it forwards the registers it
received, observes the card, and writes `ownershipFlags` only when all of the
following hold:

- `InspectOverview` accepts the object (non-zero appId, `gameId & 0xFFFFFF`
  equals appId, both flag words inside the published masks);
- the probe latched: two distinct apps matched the same layout this session;
- the app is configured by a `.lua` and Steam did not provide it as genuinely
  owned or family-shared;
- the owned bit is not already set.

The write happens on the client's callback thread; nothing queues work or
touches the object from a worker, so thread affinity is by construction. A
steamui table that publishes `[abi.CSteamApp.*]` is checked against the compiled
offsets before the first hook is armed: a contradiction latches the struct guard
and disables the write for the session instead of adapting.

### App recheck after a pin change

A version apply rewrites the Lua and nothing else, so an app whose pins moved
may keep the build the client already has until something asks it to look
again. Only those apps are ever named: an app whose pins did not change is never
rebuilt, and the contract behind both halves is under
[Pending version changes](#pending-version-changes). `RequestAppRecheck` is that
request: it queues the ids a pin comparison selected and returns; the caller is
the thread that wrote the Lua and never touches a Steam object.

The queue (`RecheckQueue`, 64 slots) is drained on
`CSteamUIAppControllerRunFrame`, at most 8 ids per frame, each handed to the
client's own `MarkAppChange(source, app, AppInfoOrConfig)`. The source is the
app-list owner, learned from the latest `MarkAppChange` the client makes on its
own: that detour stores the latest `self` into an atomic — a rebuilt UI or an
account switch replaces the owner — and forwards its arguments unchanged, so the
hook cannot alter what the client records. No id is ever submitted without that
owner — a queue with work and no owner waits for a later frame and reports
`no-source` instead of guessing an object. A queue that is already full refuses
the push, the refusal is logged with its count, and a refused id stays
unrechecked until that app's Lua changes again.

The drain requests a status write once it has handed at least one id to the
client, and once more the first time a frame finds work but no owner: every
write before that frame predates the calls the client made, so without these the
status file would keep describing the queue from before the reload.

`status.json` reports `steamui_attach`, `overview_fills`, `overview_writes`,
`overview_rejects` and `overview_probe` (`latched`, `unproven` or
`rejected:<reason>`), so "the card was not repaired" is diagnosable without a
debugger, plus `overview_rechecks` (ids handed to the client),
`overview_recheck_pending` (queue depth) and `recheck_state`: `ready`,
`no-source` (work queued, owner unknown) or `unsupported` (the module never
installed the `MarkAppChange`/run-frame pair).

Not this layer: card removal, size, install/update state, achievements and appid
translation. Each of those writes more of the client model and needs its own
validated field.

## Presence channels (`-showonline` / `-aetheronline`)

The `[presence]` section of `aethercore.toml` is the single source of truth for
per-app launch policy. AetherDesk writes it; the hooks read it through
`Settings::ReloadIfModified` (mtime check, no Steam restart). Keys:

- `default_mode = "none" | "showonline"` — policy for apps in none of the lists.
- `showonline_apps`, `aetheronline_apps`, `exclude_apps` — per-app overrides.
- Fixed precedence: `exclude > aetheronline > showonline > default_mode`.
  `exclude_apps` is a hard opt-out.
- AetherDesk keeps every app in at most one list (write-side mutual exclusion);
  hand-written configs still resolve deterministically by precedence.
- Legacy `-showonline` / `-aetheronline` launch-option tokens are still honoured
  for unmigrated configs, and are always stripped from the child command line
  (`h_SpawnProcess`) because rigid argv parsers crash on unknown arguments.

`showonline` rewrites only the outbound presence frames (the process is not
masked); `aetheronline` additionally masks the process as Spacewar/480. On the
wire the server validates the low 24 bits of `game_id`, never broadcasts an
appid the sender has no license for, and recycles `game_extra_info` into
`Friend.game_name`. For masked sessions Aether hides the real appid and recovers
it on the viewer's machine from, in order:

1. the `game_extra_info` suffix `"<name> | <appid>"` (ASCII; the invisible
   format-character variant was retired — the friends font renders tofu);
2. the same appid in `game_id` bits 32-63 (the server does not forward
   `game_data_blob`);
3. a title reverse lookup against the local library;
4. the legacy lobby-corroborated local-session fallback.

The self entry is never rewritten: doing so tears down the Spacewar session that
backs `aetheronline`. Unknown appids need a `ClientPICSProductInfoRequest`
(8903) to fill the local AppInfo cache, and the name probe is SEH-guarded.
