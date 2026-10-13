# Definitive `-showonline` plan — exact appid via `game_extra_info` ("suffix" version)

> Merges the work of the two previous phases (measurements in `04-showonline-plan.md`)
> with the transport decided after the 24/08 live test. Replaces and obsoletes
> the `aether_appid` rich-presence channel (never relayed back by the server).

---

## 1. Measured facts (design basis)

1. **The CM never broadcasts unlicensed appids.** Whatever `game_id` is sent — App,
   GameMod, Shortcut or P2P type — the server validates the appid in the low 24 bits
   and drops the whole `ClientGamesPlayed` with no license (Phase 0/1).
2. **With the 480 mask (i.e. `-aetheronline`) presence travels**, and the
   `game_extra_info` text we send comes back to friends inside `Friend.game_name`
   (measured: `app=480 gameid=480 name='The Stanley Parable: Ultra Deluxe'`).
3. **Rich-presence KVs are not a reliable channel**: the 24/08 test (staged and
   sender-side "delivered" `aether_appid` KV, never seen on the friend side) closes
   that road. `game_extra_info`, instead, is the only datum we know always arrives.
4. **The icon rebuilds on the viewer's PC**: the historic photo (CK3 icon) was
   produced by the friend's client, not the server. The working method of the other
   build — reverse title → appid lookup on the local Lua library + persona entry
   patch — is user-confirmed: with the game present (or .lua) on both PCs, name
   and image appear.
5. **Rewriting your own entry kills the Spacewar session** (measured regression):
   the friends patch always skips `friendid == selfId`.
6. **PICS metadata is not license-gated**: `ClientPICSProductInfoRequest` (8903)
   gets answered for any appid; 8904 responses already flow on the wire. It serves
   to fill the watching PC's AppInfo cache when the app was never seen (otherwise
   the icon has nothing to render from).

## 2. Definitive channel: appid suffix in `game_extra_info`

For every masked session (`-aetheronline`, or `-showonline` — where the process
stays real and only the presence frame is rewritten), the sender writes:

```
game_extra_info = "<displayable name> | <decimal appid>"
```

- The server recycles it into `Friend.game_name`: the friend PC reads it **as
  guaranteed text**, not as KVs (cf. facts 2 and 3).
- The name stays readable for vanilla friends (they see the whole string: accepted
  tradeoff, the name is the important part and comes first).
- `presenceCustomGameName` still applies as the name override; the appid suffix
  is appended anyway.
- `" | "` separator parsed **from the last occurrence**: colons and pipes inside
  titles break nothing (harness-tested: 14/14 cases).

### Watching side (`PersonaInject`, entries with `game_played_app_id == 480`)

Resolution chain, in order:

1. **`" | <appid>"` suffix** from `game_extra_info`/KVs or `Friend.game_name`
   (exact, language-independent, no collisions).
2. **Title reverse lookup** (`friend_appid_from_name`, existing method, configured
   library only: case/space-insensitive, cache with negatives) — covers senders
   without suffix.
3. **Local legacy** (`aetheronline_persona_patch` + `aetherOnlineRealAppId`) —
   pre-existing behavior, unchanged.

Once patched (`game_played_app_id`, `gameid`, `game_name` = title from the local
AppInfo cache, else the suffix's cleaned text, else emptied field so the stock
UI resolves on its own). When the local AppInfo cache **cannot even name** the
app → a `ClientPICSProductInfoRequest` is originated (8903, once per appid per
process) through the new `PacketRouter::SendClientFrame`, so the response fills
the standard cache and the icon can appear.

Your own entry is **always skipped** (fact 5).

## 3. Protected invariants

- The **self** entry of local presence stays owned by `presenceInjectLocal`
  (real game in local view), never by the friend-patch.
- `-aetheronline` (masked process + multiplayer payload) **wins** over `-showonline`
  when both flags are present.
- `-showonline` keeps NOT touching the process: achievement/DLC/cloud/screenshot/
  overlay stay identical to a flagless launch.
- **Owned** games: no masking, no suffix — the CM already broadcasts correctly
  (fact 1); touching anything would be a regression.

## 4. Files touched (vs `5dc0f2f`)

| file | contents |
|------|-----------|
| `hooks/wire/GamesPlayedModule.cpp` | inline `-showonline` rewrite (game_id → 480 + suffixed extra_info); suffix also for `-aetheronline` 480 entries; `[DIAG] TX` change-triggered |
| `hooks/wire/PersonaInject.cpp` | suffix → by-name → legacy chain; skip self; `EnsureAppInfo` (PICS); `[DIAG] SERVER self-push` + self/friend KV dump |
| `hooks/wire/PacketRouter.{h,cpp}` | `SendClientFrame` (originated frames with cloned header and zeroed job ids); connection capture; `[DIAG] SharedLibrary`; `[DIAG] BUILD showonline-suffix` stamp |
| `utils/GameNameResolver.{h,cpp}` | `ResolveAppIdByName` (configured-library reverse lookup, ± cache) |
| `core/Settings.{h,cpp}` | `presenceShowOnlineBroadcast`, `presenceFriendAppIdFromName` (default ON) |
| `core/Constants.h` | `kShowOnlineFlag`, `kExtraInfoAppIdSep` |
| `core/AetherCoreState.h` | `showOnlineAppId` |
| `hooks/steamclient/AetherOnlineHooks.{h,cpp}` | `-showonline` flag detection (no masking) |
| `diagnostics/StatusWriter.cpp` | `showonline_appid` dump + toggle |
| `proto/steam_messages.proto` | `process_id=9`, `game_flags=11`, `owner_id=12` (numbers verified against SteamKit; owner_id reserved for the Family Sharing road) |
| `config/aethercore.example.toml` | documentation of the two keys |
| `AetherDesk` (`steam.rs`, `main.rs`, `LibraryGameActionsModal.tsx`) | "Show Online" action + mutual exclusion with `-aetheronline` (round 1, unchanged) |
| `docs/04-showonline-plan.md` | the other phase's measurements (reference) |

Removed all components of the `aether_appid` KV channel (ShowOnline module, 7501
merge, dedicated KV1 codec, broadcast counters) and the typed-CGameID experiment
(falsified: do not propose it again).

## 5. Tests to run (send logs if anything fails)

Setup: `[log] level = "debug"` (or `info` for the minimum). Log `<Steam>\aethercore\main.log`.

- **T0 build check**: main.log must show `[DIAG] BUILD showonline-suffix | ...`
  — if missing, the loaded DLL is NOT this build (DESK's updater says 0.9.10 for
  both builds: do not trust that number, **do not launch auto-update**).
- **T1 sender (`-showonline`)**: `ShowOnline session for app <id>: process NOT masked`;
  `[DIAG] TX[0] game_id=... (app=480) extra='<Name> | <id>'`;
  `showonline: games_played <id> -> 480 (extra_info ...)`. Achievements/DLC/cloud untouched.
- **T2 sender (`-aetheronline`)**: as before + `[DIAG] TX` with `extra='<Name> | <id>'`;
  zero multiplayer/DLC regression.
- **T3 friend (two befriended accounts, open friend list)**: on the friend PC look for
  `Patched friend <steamid>: 480 -> <id> (extra_info)`; friend list with real name and
  icon. If the game was never seen by the friend PC, also expect
  `[DIAG] app <id> missing from local AppInfo cache; requesting PICS product info` +
  `SendClientFrame eMsg=8903 ... -> ok`.
- **T4 friend without suffix** (sender on old build): title-based method must still hold
  (`Patched friend ... (by name)`) when the friend's Lua library holds the game.
- **T5 session stability**: with `-aetheronline`, Spacewar must **not** drop; if it drops,
  look in the log for `[DIAG] SharedLibrary msg` (the CM ordering the stop) vs
  `[DIAG] self entry arrived as 480` (desync window, must not touch self).

## 6. Accepted limits (to validate in the field)

1. **Icon for apps never seen by the friend PC**: depends on PICS filling — the
   part to validate with T3. If 8903 gets answered but the client does not process
   the response (zeroed job id), the icon stays missing while name/appid work: a
   sign that a second receiver-side iteration is needed.
2. **Vanilla friends** see the full `Name | appid` (extra information, not broken).
3. **Zero persona pushes when unobserved**: 766 deltas arrive with open friend list/
   chat and actually-befriended accounts (no frames, no patch — not a bug).
4. **Titles ending in `" | "` + digits**: would only match the parser in pathological
   cases (a name ending exactly that way); risk negligible and covered by the 24-bit /
   digits limit.
5. **Server-side playtime** stays attributed to 480 (as always with `-aetheronline`).

## 7. Friend-client crash — "never owned game" scenario (diagnosing, 2026-08-24)

**Symptom**: with the sender on `-showonline` for a game the friend client owns
(installed or .lua) everything works; when the sender plays a title **the friend
never owned**, the friend's Steam **crashes** ~30–40 s after each presence delivery.

**Measured facts** (12:12–12:15 log, two machines):

1. The CM broadcasts the **real** appid (e.g. 1703340) for `-showonline` games owned
   by the sender: no 480 entry reaches the friend → the recovery chain and PICS
   (8903) **never run** on the crashing client. The crash is not in the newly added code.
2. Portable chronology: crash #1 ~12:13:40 (PC launches Stanley 1703340); restart
   12:14:10 (`showonline-suffix` stamp build present → correct build); login
   complete 12:14:34; last DLL trace 12:14:36.731; crash #2 within ~30 s. The
   sender **never** crashes (continuous log past 12:15:38).
3. Logger: immediate per-line flush → the log is complete, nothing lost in queue.
4. Open hypotheses: (a) Steam's vanilla path (appinfo/icon request for an app
   unknown to the client) detonated by the scenario; (b) a transport frame (Multi
   eMsg=1, jobs 146/147) not yet traced.

### 7.1 Flight recorder (build `showonline-suffix+flightrec`)

Added diagnostic logging, **no logic touched**:

- `hooks/wire/PacketRouter.cpp`: bidirectional per-frame trace (`send eMsg=…
  hLen=… bLen=… / recv eMsg=…`, one line per frame), hex signature of the first
  8 body bytes of Multi frames (`eMsg=1 (Multi) bLen=… head8=…`, previously
  silent), jobid→service-name job tracker (`[DIAG] service recv name='…' jobid=…
  eresult=… bLen=…`), build stamp bumped to `showonline-suffix+flightrec`.
- `hooks/wire/PersonaInject.cpp`: per persona frame, one `INFO` line per playing
  friend (`[DIAG] PERSONA friend <steamid> (SELF|FRIEND): app=… gid=… name='…'
  kvs=…`), deduplicated on state change only → tells *when* the friend entry
  (e.g. real 1703340) reaches the machine. Plus `TRACE [DIAG] PERSONA frame:
  friends=N bLen=…`.

Default `[log] level = "trace"` → active with no config changes.

### 7.2 Test procedure (crash)

1. Build and put this build on **both** machines.
2. PC: Steam running; launch with `-showonline` a game the PC owns and the **laptop
   lacks** (e.g. Stanley 1703340).
3. Laptop: keep Steam open, watching friends, until the crash.
4. For EACH machine attach **both** `<Steam>\aethercore\main.log` **and**
   `<Steam>\aethercore\main.log.last` (rotate on restart: the crashed session ends
   up in `.last`!) — plus the `DESK` session log when available.
5. From the **laptop**, also capture: Event Viewer → Windows Logs → Application →
   "Application Error"/"Windows Error Reporting" entries at crash time; the
   contents of `Steam\dumps\` (minidump with the crash timestamp); in Steam's
   Main.log (`Steam\logs\`), the last minute's lines.

### 7.3 Expected reading

- If an unexpected new `recv eMsg=…` or a `service recv` with eresult ≠ 1 shows
  up before the crash → wire-side culprit, look at that frame.
- If `PERSONA friend … 1703340` shows up → the entry arrives via 766 (not Multi);
  crash within seconds after → likely vanilla consumer (appinfo/icon).
- If the crash kills the trace with no suspicious frames → failure outside the wire
  (UI thread): dumps/Event Viewer point at the real module; remove the flight
  recorder after diagnosis.

### 7.4 Crash culprit — FOUND AND FIXED (2026-08-24, `+flightrec` build)

New logs: **working** recovery chain on Gamblers Table (`Patched friend
76561199876393402: 480 -> 3618390 (extra_info)`), immediate crash on Stanley:

- 12:57:21.863 persona delta friend: `app=480 name='The Stanley Parable: Ultra
  Deluxe | 1703340'` (PC had transmitted 480+suffix at 12:57:20.089).
- **No** line after: died inside `gamename::ForApp(1703340)`, i.e. inside the raw
  `CAppInfoCache::GetAppDataFromAppInfo("common/name")` call on an appid **with
  no local record**: on this Steam build that probe faults and kills the process
  with no catchable exception from the flush.
- Entries for apps known to the cache (e.g. installed 3618390) probe fine → which
  is why the chain worked with games present on both machines.

Applied fix:

1. **`PersonaInject`**: the display name takes the suffix text (minted by the
   sender from THEIR AppInfo cache — fresh and localized). `ForApp` stays only
   for the "local session" source (locally installed app → proven-safe probe).
   No more live probes for apps outside the recovery path.
2. **`GameNameResolver::ForApp` (SEH-guarded)**: the raw probe is wrapped in
   `__try/__except` (MSVC): a fault becomes "unknown name" with an
   `appinfo probe ... raised SEH 0x...` log instead of the crash — also serving
   as the ultimate truth if the hypothesis were wrong.
3. **PICS 8903 on a detached worker thread** (+50 ms, `PICS appinfo send
   begin/done` TRACE parens): no re-entry of `BBuildAndAsyncSendFrame` inside
   the RecvPkt stack.

Expected on T3 retest (Stanley): `Patched friend ... (extra_info)`, then
`[DIAG] app 1703340 missing from local AppInfo cache; requesting PICS`, then
`[DIAG] SendClientFrame eMsg=8903 ... -> ok`; icon filling in once cached.

**FIX CONFIRMED (13:19–13:27 log, `+fix2` build)**: laptop patches and requests PICS
for every unknown app without crashing — `Patched friend ... -> 3618390/1703340/1398210
(extra_info)` + `SendClientFrame eMsg=8903 ... -> ok`; Steam alive until export.

## 8. GAME crash with `-showonline` on rigid argv parsers (2026-08-24, `+fix3`)

**Symptom**: "Selene ~Apoptosis~" (1398210) exits ~3–4 s after every launch with
`-showonline` (3 launches between 13:21:31 and 13:21:57, 4.6 s / 4.2 s lifetimes),
while the flagless launch lives 28 s. Gamblers Table and Stanley tolerate the flag — Selene doesn't.

**Cause**: with `-showonline` the process is NOT masked, receives no payload nor env
patch — the **only** difference visible to the child vs a plain launch is the
`-showonline` token left on the command line (SpawnProcess passes `cmdLine`
unchanged). Games with rigid argv parsers (or treating unknown args as paths) crash.
**Fix** (`hooks/steamclient/AetherOnlineHooks.cpp`): `h_SpawnProcess` now strips Aether
tokens (`-aetheronline`, `-showonline`) from the cmdline before calling
`o_SpawnProcess` → the child gets a clean argv (log: `Stripped Aether launch flags
from child cmdline (app <id>, was '...').`). Bonus: same protection for `-aetheronline`.
`HasFlagArg` tokenization made consistent (space+tab). Harness tests 14/14.

## 9. Invisible channel for the suffix (build `+fix4`, 2026-08-24)

**Motivation**: with the ASCII `"Name | appid"` suffix, friends WITHOUT Aether see
the whole string. Ask: show vanilla only the name, without losing deterministic
appid reception.

**Proposed idea (braille): discarded** — braille characters (U+2800–U+28FF) are VISIBLE
(dots) in Windows fonts. Adopted solution: **Default-Ignorable format characters**,
invisible by Unicode definition in every conforming renderer (the friends CEF UI
included): tag = `U+200B` ZERO WIDTH SPACE (E2 80 8B) + exactly 6 **Variation
Selectors** U+FE00..U+FE0F (EE B8 8n), one nibble each, 24-bit big-endian. U+200B
detaches the VS chain from the last visible character (no restyle of digits/®/© with
FE0F). 21 bytes overhead.

| Side | File | Behavior |
|---|---|---|
| Sender | `GamesPlayedModule::WithAppIdSuffix` | `presenceSuffixInvisible` (default **true**, toml `suffix_invisible`) → invisible form; `false` → ASCII legacy |
| Receiver | `PersonaInject::AppIdFromSuffix` | tries ASCII **then** invisible channel → old and new builds understand each other |
| Config | `Settings.h/cpp`, `aethercore.example.toml` | new `suffix_invisible` flag |

Round-trips harness-tested (`/tmp` 22 tests, ALL PASS): unicode/CJK names, trailing ® and
digits, leading-zero appids, 480/0 rejected, malformed tail rejected, ASCII back-compat.

**Remaining limits**: (1) Aether receivers < `+fix4` don't read the invisible channel
(fallback: title match, as before); (2) if the CM ever normalized/scrubbed DICPs
from game_name, the invisible suffix would vanish → by-name fallback: to verify in
the field (T3).

---

## 10. `game_data_blob` channel (fix5) — appid invisible to vanilla, with no text at all

### Field measurement closing §9
The invisible-character channel FAILS live: Steam's friends UI rasterizes
U+FE00-FE0F as tofu rectangles (the font used for the game line lacks those glyphs —
2026-08-24 measurement, `all.log` + user confirmation). Any **textual** encoding depends
on the renderer's font: the definitive fix = a **non-textual** channel.

### The field
The complete Steam protobuf set (verified against LumaCore/source/proto) defines
| message | field | no. | type |
|---|---|---|---|
| `CMsgClientGamesPlayed.GamePlayed` | `game_data_blob` | 8 | bytes |
| `CMsgClientPersonaState.Friend` | `game_data_blob` | 60 | bytes |

Raw bytes, NEVER rendered by any client UI (stable/beta/mobile Steam). Like
`game_extra_info`→`game_name`, the CM recycles `game_data_blob` into friends'
Persona frames → a mirror channel but 100% invisible.

### Format (9 bytes)
`"AETR"` (4) + version `0x01` (1) + LE appid (4). Strict decoder: length, magic,
version, 0 < appid ≤ 0xFFFFFF, appid ≠ 480. Foreign blobs (other games/emulators
genuinely using it) are dropped with no effects.

### fix5 flow
- **TX** (GamePlayed): `AnnotateMaskedEntry()` — with `appid_blob=true`,
  `game_extra_info` = **plain name only** + `game_data_blob` = blob; both paths
  (-showonline rewrite and -aetheronline annotate).
- **RX** (PersonaState): after suffix parses, before by-title fallbacks:
  `AppIdFromBlob(f.game_data_blob)` → `source="blob"`, displayName = cleaned extra
  text. Vanillas see **only the name**: the user ask ("no `" | appid"`, no tofu")
  holds by definition (the CM sanitizes `extra_info` with EXACTLY the app's name →
  no textual leak possible).
- **Default policy**: `appid_blob=true`, `suffix_invisible=false` (retired:
  measured tofu), ASCII suffix available only with `appid_blob=false` for legacy
  receivers (<fix3 doesn't even have that).
- **receiver legacy** (<fix3 doesn't even have that).
- **Default policy**: `appid_blob=true`, `suffix_invisible=false` (retired:
  measured tofu), ASCII suffix available only with `appid_blob=false` for legacy
  receivers (<fix3 doesn't even have that).

### Required field test (single proof point)
The CM's behavior on `game_data_blob` is UNDOCUMENTED: proof is needed that it
gets recycled like `game_extra_info`. Fix5 build, search the laptop's RX log for:
- `[DIAG] BUILD showonline-suffix+fix5 ... appid_blob=1` (startup stamp)
- a `Patched friend <name>: gameid -> <appid> (blob)` line or a "blob" diagnostic
  in the source (added `source="blob"`) → the CM **forwards** the blob: OK case.
  The laptop sees the real appid.
- **If instead** the CM drops the blob (presence falling back to by-title/by-name):
  move to plan B (`Friend.gameid` field 56 fixed64 — bits 32-63 free when
  type=mod: pack the appid there; vanillas see Spacewar, Aether decodes the high
  bits; collision acceptable only with genuine Spacewar mods).

### Considered edge cases
- **AM extra_info sanitization**: irrelevant for the appid (no longer traveling
  in text); the clean name still arrives (the CM recycles it as game_name).
- **Persona frame merge**: `FILL` spec for infuse frames (own+friends) honors
  unknown→f.relay via dedicated proto field 60: the aggregator code never
  touches `game_name`, the decoder reads the direct field.
- **Future version**: byte 4 reserved; bump to `AETR\x02`→ legacy decoder
  rejects and falls to by-title (graceful).
- **RX DIAG log**: `Patched friend ... (blob)` encodes the field proof;
  `showonline` TX logs `channel=blob`.

### Field measurement #2 (fix5 build, 2026-08-24 16:38) — the CM does NOT forward the blob
`pc_all.log_16-42-51.txt` (TX, fix5, blob active on every game) and
`portatile_all.log_16-42-27.txt` (RX, fix5): the receiver sees the mask
(`PERSONA friend ...: app=480 gid=480 name='DAVE THE DIVER'`) but recovery fails
across the whole chain (blob included) → **the CM does not recycle
`game_data_blob` into Persona frames**. The (text) extra_info channel stays the
only actually-recycled one; hence the vanilla-friendly update: vanillas see the
clean name ✓, but Aether friends no longer recover the appid.

### Plan B activated (fix6): appid in game_id bits 32-63
- **TX**: on the -showonline path `AnnotateMaskedEntry(packGameIdHighBits=true)`
  writes `game_id = (480 | saved typeBits) | (appid << 32)`. The -aetheronline
  path does NOT touch gid high bits (risk to AetherOnline discovery).
  Unchanged for vanillas: the UI keys on bits 0-23 (480) + extra_info text; mod
  bits are never rendered.
- **RX**: after blob, before by-title: `gidHi = gameid >> 32` validated
  (0 < gidHi ≤ 0xFFFFFF, ≠ 480) → `source="gameid"`.
- **Definitive RX DIAG** (one line per masked friend):
  `[DIAG] mask friend <id>: gid=<gid> gidHi=<hi> bloblen=<n> blobhead=<hex> extra='...'`
  — decides with no ambiguity which channels the CM really forwards.
- Residual risk: the CM might zero the mod bits in the relay (the
  `PERSONA friend ... gid=480` of the fix5 test proves NOTHING: high bits in the
  fix5 TX were 0 by construction). If gidHi arrives, the channel is definitive;
  otherwise the chain falls to by-title (known pre-fix1 behavior).
- Harness: `/tmp/gid_tests.cpp` (8/8: 480 rewrite preserves low, hi decode,
  hi=0/spacewar/>24-bit rejection).

---

## 11. Crash class: games dying on launch arguments (`showonline_apps` marker, fix7)

### Why Selene and Z.A.T.O. crashed
`-showonline` was written into Steam's **Launch Options** (permanent), hence
present in argv on every launch.
- **Selene ~Apoptosis~**: rigid argv parser — exited 3–4 s after EVERY flagged
  launch (2026-08-24 measurement, three runs). Fix B: token strip in
  `SpawnProcess` (`StripAetherFlagArgs`).
- **Z.A.T.O.** (app 4122860): the strip DOES happen correctly
  (`pc_all.log_17-05-07.txt` l.1219: `Stripped Aether launch flags ... ZATO.exe
  -showonline`), the child receives a clean `'"ZATO.exe"'`, yet dies ~2.5 s
  after launch. Conclusion: the argv surface was not the (only) road — the game
  also reads launch options through another surface (e.g.
  `ISteamApps::GetLaunchCommandLine`, which Steam feeds from the REGISTERED
  launch option, not the child's argv — our strip never touches it).

### Structural prevention (fix7): no marker on the command line. EVER.
The only reliable defense against the WHOLE class is touching no surface
visible to the game:
- **AetherDesk**: the showonline toggle writes the appid into
  `[presence] showonline_apps` in **both** aethercore.toml copies (local config
  dir + `<Steam>/aethercore/aethercore.toml`). No more `-showonline` in Launch
  Options; legacy tokens are removed on first toggle (migration in
  `set_aether_showonline`), and the reader (`get_aether_showonline`) recognizes
  marker OR legacy token for compat. Enabling the marker also removes
  `-aetheronline` (mutual exclusion, unchanged).
- **AetherDLL**: `h_SpawnProcess` calls `Settings::ReloadIfModified` (mtime; no
  Steam restart) and activates the session when `realApp` is listed. Env block,
  blob, argv, workdir stay byte-identical to a normal launch: the "arguments"
  crash becomes impossible by construction.
- Spawn log naming the source: `(source: showonline_apps marker (clean argv))`
  vs `(source: launch arg (legacy; strip applied below))`.

### Expected field verification
Enable showonline on Z.A.T.O. from the AetherDesk UI (marker), launch: expect
life > 2.5 s (no crash). If it crashes EVEN with no arguments in Steam's launch
options, the problem is inside the game/environment and out of Aether's reach
(test: put any `-test123` in the launch options with NO Aether active — if it
crashes, the game is simply fragile to launch arguments, a known trait of it,
and the marker stays the only sane road).

---

## 12. Centralized config in TOML (fix8): policy + overrides, zero argv

### The adopted professional model
- **TOML as the single source of truth** (`[presence]`): the three arrays
  `showonline_apps`, `aetheronline_apps`, `exclude_apps` + the
  `default_mode = "none" | "showonline"` policy.
- **Policy + overrides, NOT enumeration**: "every unconfigured game shows
  presence" = ONE `default_mode = "showonline"` line instead of materializing
  the whole Steam library into the array (anti-pattern: derived data written as
  intents, pointlessly huge file, multiple writers, install/uninstall leaving
  garbage).
- **Fixed, documented precedence**: `exclude > aetheronline > showonline >
  default_mode`. Wired in `ResolveLaunchMode()` (h_SpawnProcess); C++ harness
  12/12 (including: exclude beats even a forgotten legacy token).
- **`exclude_apps` = hard opt-out**: the DLL ignores the app entirely; residual
  argv tokens still get stripped.
- **Write-side mutual exclusion**: every Desk command removes the app from all
  arrays and adds it to at most one → dirty "app in two lists" configs never
  form; hand-written ones resolve by precedence order (deterministic).

### AetherDesk (fix8)
- The four existing commands keep names and signatures → the UI is unchanged.
  `set_aether_showonline` / `set_aetheronline` now write into the toml arrays
  (both copies) and **always remove legacy tokens** from Launch Options
  (progressive migration).
- New registered commands: `get/set_aether_excluded` (opt-out) and
  `get/set_presence_default_mode` (global showonline-default policy).
- Generalized TOML editor: canonical upsert of the three keys under `[presence]`,
  section creation when absent, everything else preserved; Python mirror tests
  11/11 (idempotence, canonical order, dedupe, append).
- The DLL does ReloadIfModified in SpawnProcess → **zero Steam restarts**.

### Answer to the crack question (online-fix.me / UCO2)
User observation: `-showonline` worked with cracks that THINK they are Spacewar.
Architectural reason: showonline never touches the process — it only rewrites
outbound presence frames — so it coexists with any local masking. But:
- **-showonline + -aetheronline on the SAME game must not be combined** and is
  pointless: the AetherOnline mode (real 480 mask) is a functional superset —
  friend-facing presence is included in its annotation path. Mutual exclusion
  (one array per app) is the correct shape.
- With a crack masking ITSELF (without Aether `-aetheronline`): the showonline
  annotation wiring never fires (requires `luadata::IsConfigured`/depot):
  friends simply see "Spacewar". For name+appid to friends on such a game, use
  `aetheronline_apps`.
- "The multiplayer pipeline should not be busy": correct for UCO2 — showonline
  reserves nothing process-side; the only shared zone is the presence wire
  channel, owned by the CM.

### User procedure after the update (migration)
1. Rebuild DLL + Desk.
2. In AetherDesk: re-toggle (off→on) every previously marked game → tokens
   vanish from Launch Options and arrays appear in the toml.
3. For global plug-and-play: `set_presence_default_mode(true)` (future UI
   toggle) or by hand `default_mode = "showonline"` in the toml.
4. Log check: `showonline-suffix+fix8` stamp; at spawn a line with
   `(source: showonline_apps|aetheronline_apps|default_mode=showonline|...)`.

---

## 13. Integrated Desk UX (fix9): default policy, 4-way popup, custom name, Steam monitor

- **Bundled defaults**: `AetherDesk/src-tauri/assets/defaults/aethercore.toml`
  now holds `[presence]` with `default_mode = "showonline"` + 3 empty arrays
  (fresh installs). `core/migration::ensure_aethercore_bridge` calls
  `presence_config::ensure_defaults` on every boot → missing keys inserted on
  existing installs, user choices NEVER touched (8/8 tests).
- **Shared module**: `core/presence_config.rs` gathers ALL [presence] TOML
  persistence (extracted from the commands: DRY/separation).
- **ONLINE popup = segmented control**: None (= exclude_apps, hard opt-out) /
  Show Online / Online Aether (radio, mutual exclusion; the active mode is
  marked ACTIVE, clicking the active one = no-op) + orthogonal UCO2 toggle from
  its panel. Layout: **1 column × 4 rows** (full-width vertical list, fixed
  110px title + hint on the right). Rules: Online Aether blocked when UCO2
  deployed; UCO2 blocked when Online Aether active. The shown mode is the
  EFFECTIVE one (falling back to default_mode when the app is in no list).
  Detection: 4 parallel state invokes + UCO2 state from the canonical panel.

### Live fix9 test findings (24/08) → applied corrections

- **UCO2 "always active"**: the popup gating used `is_uco2_active`, a file probe
  on `union-crax.ini` — a RUNTIME file UCO2 leaves on disk even after
  Disable/undeploy → a perennial false positive blocking Online Aether. Truth
  source now = `get_online_status` (OnlineEngine::status: record in
  uc_online2.json + ini/dll on disk; `broken`/unconfigured = inactive), the same
  view as the Enable/Disable panel. Fix in
  `LibraryGameActionsModal.refreshOnlineStates` (typed `OnlineStatus` invoke
  imported from `./OnlinePanel`, tsc strict clean).
- **Stale Start/Restart Steam label**: hardened monitor — `lifecycle` logs at
  startup and on every state transition (diagnosis via desk.log),
  synchronous `mark(false)`/`mark(true)` inside `restart_steam` after
  successful kill/spawn (UI reactive at once; the 2.5 s poller confirms/corrects),
  defensive `is_steam_running` re-read 2 s after the click in App.tsx. Event
  wiring verified end-to-end (App.tsx l.219 → Sidebar).
- **Settings**: "Show every game to friends by default" switch moved under
  "Enable test updates", above "Custom game display name" (requested order).
- **ONLINE-popup-EXCLUSIVE presence choice**: removed the direct "Show
  Online/Disable Show Online" button (and its handler — dead code) from the
  Modify popup: presence is decided only from the ONLINE popup.
- **Desk-driven ShowOnline inert while `-showonline` (argv) works**: desk.log
  proves the writes landed (toml marker, default_mode). Primary hypothesis =
  **pre-fix9 installed DLL**: the PE version stays pinned 0.9.10 and cannot
  tell builds apart; mandatory check = rebuild AetherDLL and look for the
  `[DIAG] BUILD showonline-suffix+fix9` stamp (l.~405 PacketRouter.cpp) in the
  current session's main.log. Missing stamp = markers written but an old DLL
  that never reads them.
- **Verdict line for EVERY launch** (AetherOnlineHooks::h_SpawnProcess): before,
  the mode==None / depot-miss branch was SILENT — no proof of why the DLL
  ignored an app. Now every launch logs
  `Presence resolve: app <id> -> none|showonline|aetheronline (source: ...;
  argv aetheronline=N showonline=N; depot=N)`: marker vs argv token vs default
  reading made explicit on every boot, definitive diagnosis with no more hypotheses.

## 14. Root cause found (fix10): INVALID bundled TOML + silent failure circuit

Live test 24/08: markers written but NEVER honored, logs stuck at WARN level
although `[log] level = "trace"`. Definitive diagnosis with real toml++ v3.4:

- **Root cause**: the bundled `AetherDesk/assets/defaults/aethercore.toml` asset
  declared `[presence]` TWICE (bad merge in fix9, with Italian comments). TOML 1.0
  forbids table redefinition → `toml::parse_file` throws
  `parse_error: cannot redefine existing table 'presence'` → `Settings::Load`
  catch → ALL defaults: in Release `logLevel=Warn` (hence warnings only), empty
  presence lists, ignored `default_mode` → Desk wrote, the DLL read nothing. argv
  `-showonline` worked because it never touches the config. Commit 4e5ebf5
  irrelevant: regression introduced in fix9, never existed before.
- **Broken silence circuit**: the catch logged "No usable config" at INFO, filtered
  by the very Warn default the failure caused. Now: `AC_LOG_WARN` with
  `parse_error.description()` (always visible), `Loaded` summary with level +
  presence list counts.
- **Automatic on-disk file repair** (presence_config `dedup_sections`, called from
  `ensure_defaults` on every Desk boot): duplicate sections fused into the first
  occurrence (duplicate's unique keys moved to the first's tail; on key ties the
  first wins; duplicate block comments dropped). User `aethercore.toml` files
  self-heal on the new build's first run, choices preserved.
- **Full hot-reload**: `ReloadIfModified` now also reapplies `log::SetLevel`
  (Desk-side level change = effect at next launch, no Steam restart) and the mtime
  comparison uses the timestamp captured BY `Load` (before, the first call was a
  mute warm-up → the first game launched after a Desk edit resolved on stale
  settings).
- Asset fixed: single `[presence]`, **all in English**, validated with tomllib
  and toml++.
- Tests: python mirror 15/15 (real corrupt file repair: 1158310 marker, custom name
  and user default preserved; idempotence; legacy upgrade; no-op on healthy asset)
  + C++ gold with real toml++ v3.4 6/6 (the corrupt file throws exactly
  `cannot redefine existing table 'presence'` → defaults).
- Build stamp: `showonline-suffix+fix10` (to tell the fixed DLL apart).

## 15. Masked Meccha session (~5 s crash) + friend attributed to the wrong game

Live test 25/08 with working trace logging. Facts measured in the log:

- **Resolver/mask intact**: `Presence resolve ... -> aetheronline (aetheronline_apps)`,
  `Masked AppId 4704690 as Spacewar (480)`, overlay 480->4704690, donor stats
  spoof active on every launch. The TOML marker channel (fix9-10) works.
- **The game process NEVER handshakes the IPC pipe** (no PipeWatch line for
  PenguinHotel.exe); dies at ~5.3 s in EVERY masked launch, and at ~8 s even with
  a clean showonline launch (tokenless argv). Machine Party (UCO2 game) handshakes
  regularly (`source=SteamOverlayGameId env=480`) and lives: death is inside the
  game's init, not the network wire.
- **HEAD vs fix-series diff on the mask path = superficial**: SpawnProcess mask,
  env block, scoped GetAppID, wire egress OF (extra_info/blob), staged
  persona-inject (inject_local), RP upload: all identical or also present in HEAD,
  when Meccha worked. No demonstrable regression there.
- Main open hypothesis: Spaceworks/480 environment on the PC or Steam build
  1785799196 — outside the fix-series. Two-minute discriminating test (toml,
  hot-reload at launch, no Steam restart): `[presence] inject_local = false`
  (then, if insufficient, also `always_extra_info = false`). If Meccha LIVES with
  inject_local=false → the culprit is self-inject during the masked session and
  must be hard-gated (skip when aetherOnlineRealAppId != 0). If it dies the same →
  investigate the machine's 480/licenses side, not the persona code.
- **`legacy_tokens_found`**: Desk NEVER writes argv tokens (markers only); the
  `-aetheronline` seen in 40:11+ launches were added by hand (the DLL strip
  removes them anyway). desk.log label fixed (was `legacy_tokens_removed` but
  printed "found").

### Friend shown as "MECCHA CHAMELEON" without ever installing it — ROOT CAUSE + FIX

The legacy "local session" fallback (pre-suffix) attributed EVERY friend with
app=480 and no recoverable appid to OUR aetherOnlineRealAppId, given an active
Online Aether session. Measured in the log: a friend in their own unrelated
masked session → shown as MECCHA CHAMELEON on the user's client. Fix (fix10):
legacy attribution now requires **lobby corroboration**: the friend's
`steam_player_group` equal to ours (tracked from the SELF entry on every push,
atomic `g_selfLobby`) and non-zero. No shared lobby = no guessing: the entry
stays 480 (vanilla view) — a false attribution is always worse than a "Spacewar".
The exact channels (suffix/blob/gid-hi/by-name) stay unchanged and primary.
Implementation: new `LobbyGroupId(Friend)` helper, `fromLocalSession` flag
instead of `source`-string comparisons (rename-proof).
- **Settings → Aether**: "Show every game to friends by default" switch
  (set/get_presence_default_mode, immediate apply, no Save).
- **custom_game_name**: in [presence] now has ARCHITECTURAL precedence — the app
  resolves to ShowOnline in the resolver with no explicit entries
  (exclude/aetheronline untouched; the name still wins in both pipelines via
  DisplayName). "Lightened" behavior confirmed: never masks.
- **Steam monitor** (`core/steam_monitor.rs`): dedicated poller (2.5 s) +
  AtomicBool + `steam://runtime-state` event on changes only →
  `is_steam_running` O(1); Sidebar shows "Start Steam" closed / "Restart Steam"
  open. The restart_steam command was already idempotent (kill=no-op when
  absent → pure start); only the label changes.
- Resolver C++ harness updated: 16/16 (custom-name + all precedences).
