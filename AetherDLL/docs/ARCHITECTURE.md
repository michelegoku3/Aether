# AetherCore Architecture Notes

This document records the invariants that keep AetherCore simpler and more maintainable than the LumaCore/OpenSteamTool code it replaces.

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

All disk I/O runs on a dedicated lazy-started worker thread owned by
`AchievementBackup` (`RecordUnlock()` only enqueues — no filesystem work on
Steam's network thread). `FlushOnShutdown()` is BLOCKING: it drains the queue,
copies the `.bin` files and joins the worker. It is only called in the explicit
off-loader-lock shutdown path, NOT automatically on normal Steam termination.
Even when called, it cannot guarantee Steam has flushed its cache. Event-driven
checkpoints/game-exit persistence remain a separate follow-up. The JSON is rewritten atomically on each
unlock (merge keeps the earliest unlock time). If the Steam cache is ever
wiped again, restore by copying the `.bin` back into `appcache\stats` (Steam
closed) or rebuild it from the JSON with `Tools/achievement_decoder.py rebuild`.

`Tools/achievement_decoder.py` decodes/encodes the Steam cache, schema and
mirror JSON files (`decode`, `snapshot`, `rebuild` commands) — use it together
with `main.log` when investigating achievement issues.


## Runtime publication and lifecycle (quick-win batch)

* Settings are immutable `shared_ptr<const Settings>` snapshots, atomically
  published. Each reader retains its owner for the duration of access. Only
  DirWatch polls the TOML at runtime (about 1 s, including without Lua watches).
  Invalid edits retain the last good snapshot. Lua watch-directory changes need
  a restart; changing settings does not rebuild every initialized service.
* HookManager serializes registry operations; `Snapshot()` returns owned copies.
  Bootstrap still serializes whole batches. Status requests only increment an
  atomic counter; one writer coalesces them at 100 ms intervals. Metadata uses
  `statusMetadataMutex`; runtime counters keep their existing locks/atomics.
* IPC maps are published ONCE before setting atomic `loaded`; no reader may
  touch the maps before acquiring that flag. Init attempts are serialized.
* Pattern indexes are built locally on late retry, then published under
  `patterns.mutex`; lookups/availability checks use a shared lock.
* DllMain detach only sets the shutdown flag: no joins, logging, mutex-taking
  flush, hook teardown or persistence. Logger already flushes each written line.
  `AetherCoreShutdown` is an explicit terminal shutdown API for a host that has
  quiesced activity, called off loader lock; no production caller is installed
  by this batch. PinSelf prevents normal FreeLibrary unloading. This does not
  replace a full worker registry/quiescence protocol and does not make arbitrary
  live unloading safe. Detached legacy jobs remain follow-up work.

## Invasive batch (event-driven persistence, workers, hot paths)

* Achievement persistence is EVENT-DRIVEN, not shutdown-driven: the backup
  worker runs a periodic checkpoint (5 min: forced .bin re-copy of every
  touched (app, account) + playtime refresh) and `SessionEnded()` schedules a
  delayed final copy (~15 s) when a game session ends. Worst-case loss on a
  hard exit is one checkpoint interval; `FlushOnShutdown()` remains for the
  explicit shutdown path and now also drains delayed jobs immediately.
* Background work lives in `core/Workers`: one task queue (one-shot jobs,
  drained and joined at explicit shutdown) plus a named worker registry with
  stop flags. There are NO `std::thread(...).detach()` calls left in
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
