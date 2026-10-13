# `-showonline` plan — showing the real game to friends (without online)

> Repo `michelegoku3/Aether` @ `5dc0f2f`. Updated after Phase 0 (log `all.log_14-15-54.txt`)
> and SyntaxCode15's friend-list photo.
> **Status: constraint measured, memory mechanism identified, Phase 1 experiment ready in tree.**

---

## 1. Phase 0 measurement — the CM validates the license (for `App` type)

| # | Game | License | Sent (`TX`) | **Server response** |
|---|---|---|---|---|
| 1 | Rivals of Aether `383980` | owned | `game_id=383980` | ✅ `app=383980` after **0.32 s** |
| 2 | Stanley `1703340` | **unowned**, no flags | `game_id=1703340` | ❌ **no push for 20 s** |
| 3 | Stanley `1703340` | unowned, `-aetheronline` | `game_id=480` | ✅ `app=480 name='The Stanley Parable: Ultra Deluxe'` after **0.46 s** |

The PersonaState the CM sends back for our SteamID is what the friend list receives. Cases 1 and 3
answer in under half a second; case 2 stays silent. That is not latency: **it is a refusal**.

**Conclusion:** with a `CGameID` of **type `App`** and an unlicensed appid on the account, the server
broadcasts nothing. Aether's ownership spoof is client-side and the CM never sees it.

**Useful side effect (run 3):** the `game_extra_info` field we send comes back as
`Friend.game_name` in the broadcast PersonaState. The real title already reaches friends even under a 480 mask.

---

## 2. The photo: what it really proves

SyntaxCode15's friend list, `michelegoku3` row:

```
[red CK3 icon]  [avatar "?"]  michelegoku3
                                  Crusader Kings III
```

Correct reading (confirmed by the user): **the square on the left is the game icon**, the red
band of Crusader Kings III. The central "?" is the default avatar (michelegoku3 has no profile
picture). Compare with the `Dreadstew` row: KH icon + black-and-white avatar.

This is decisive: **the friend's client resolved appid `1158310`**. With `app=480` that would be
impossible — Spacewar on SteamDB has `icon · empty string`, there is nothing to draw.

Setup of that era (confirmed): **no real license** on CK3, only Lua, and the **"bugged" external
aetheronline that sent the game's appid**.

### The apparent conflict, and its resolution
Run 2 says "unowned real appid → the server stays silent". The photo says "the real appid reached
the friend". Both are true **if the CGameID type was not `App`**.

```
CGameID = appid:24 | type:8 | modid:32          (Valve, steamclientpublic.h)
  type 0 = App        -> the CM verifies the license      [MEASURED: dropped]
  type 1 = GameMod    -> not an ownership claim
  type 2 = Shortcut   -> same
  type 3 = P2P
```

With `type != 0` the message **is not** the claim "I'm playing app X which I own", so the
license check does not apply — but **the real appid stays in the low 24 bits**, and the friend's
client uses it to resolve icon and name. Result: CK3 icon + real title, exactly the photo.

It also explains the rest of your memory: the **"half and half" where DLC and workshop didn't work**
is precisely this, because that aetheronline changed identity **at process level**, breaking
client-side DLC and workshop.

---

## 3. Phase 1 — DONE, hypothesis FALSIFIED

Log `all.log_18-09-37.txt` (23/08, 18:04–18:09). Build verified by the
`[Wire] [DIAG] BUILD showonline-fase1` stamp at line 150; TOML hot-reload confirmed
(the `[Settings] [DIAG] presence:` line changes every round). Six valid runs.

### Without `-aetheronline` — unowned real appid (Stanley 1703340)

| type | modid | sent game_id | CM response |
|------|-------|--------------|-------------|
| 1 GameMod  | 0x80000001 | 9223372041168223660 | **no push, 28 s** |
| 2 Shortcut | 0x80000002 | 9223372045479968172 | **no push, 19 s** |
| 3 P2P      | 0x80000003 | 9223372049791712684 | **no push, 20 s** |

The rewrite works: `[DIAG] ShowOnline: game_id 1703340 -> ...` appears on every
run, the frame leaves (`bodyLen` grows from 86 to 94/97). The CM simply **never
answers**.

Further decisive proof: the four `app=0` pushes (session end) land only at
18:04:28, 18:08:31, 18:09:06, 18:09:32 — i.e. **only around the 480 runs**.
After exiting runs 1/2/3 there is no "clear" push: the CM had nothing to zero
because **it never accepted the message**.

### With `-aetheronline` — game_id 480 (all three types, as control)

The ShowOnline block deliberately skips 480 entries, so behavior is identical
across all three rounds: CM push after ~0.2–0.3 s with
`app=480 gameid=480 name='The Stanley Parable: Ultra Deluxe'`. Name yes, icon no.

### Conclusion

**The CGameID `type` bits bypass nothing.** The CM validates the appid in the low
24 bits and drops the whole message with no license, whatever the type. The
"typed CGameID" road is closed: do not propose it again.

Together with Phase 0 this closes *every* server-side road: no `ClientGamesPlayed`
exists that convinces the CM to broadcast an unowned appid to friends.

---

## 4. Phase 2 — the right track: rewriting happens on the VIEWER's PC

If the server cannot transmit the real appid, then a month ago **it never
transmitted it**. The CK3 icon in the photo was rebuilt **locally by
SyntaxCode15's client**. The code to do it already exists in repo — it is
`PersonaInject.cpp:229`:

```cpp
// AetherOnline: patch any friend entry still showing Spacewar (local view).
if (ofPersonaPatch && ofReal != 0 && luadata::IsConfigured(ofReal)) {
    ...
    if (f->game_played_app_id() != kSpacewarAppId) continue;
    f->set_game_played_app_id(ofReal);
    f->set_gameid(ofReal);
    f->set_game_name(name);
```

It rewrites **any** friend entry showing 480, substituting `ofReal`. That is
exactly the line producing `Patched friend 76561199876393402: 480 -> 1703340`
in our logs.

### Why it is not enough today

Two constraints make it inert in the normal case:

1. `ofReal != 0` — it fires **only while you too are playing** something with
   `-aetheronline`. If you watch the friend list without playing, `ofReal` is 0 and it does nothing.
2. It uses **your** appid for **your friend's** entry. It only works in the
   symmetric case: two PCs playing the same game. Which, in all likelihood, is
   exactly the setup of a month ago.

### The correct design: resolution by name

Phase 0 measured that the CM recycles `game_extra_info` into
`Friend.game_name`. So in the incoming message the **real name is already there**,
even when the appid is 480:

```
app=480 gameid=480 name='The Stanley Parable: Ultra Deluxe'
```

The watching client has everything it needs. Just invert the map:

```
for every friend f with f.game_played_app_id == 480 and non-empty f.game_name:
    real = ResolveAppIdByName(f.game_name)
    if real != 0:
        f.game_played_app_id = real
        f.gameid             = real
```

No `ofReal`, no symmetry constraint, works with the game off.
`ResolveAppIdByName` builds from the local `appinfo.vdf` (name + `clienticon`
of every app the client has seen), with an in-memory cache.

### User confirmation

A month ago: two PCs, two different accounts, **same program and same game on
both**. That is the exact symmetric case where the existing `ofReal` block
fires. The mechanism is therefore identified: no server-side magic, just local
rewriting on the viewer's PC.

### The limit to accept

Only friends **running Aether** see the icon. Everyone else keeps seeing
Spacewar. That is not a regression: it is the maximum achievable, and it matches
what really happened a month ago.

---

## 5. Phase 2 — implemented

| file | change |
|------|----------|
| `utils/GameNameResolver.{h,cpp}` | `ResolveAppIdByName(name)`: reverse lookup over `luadata::LibraryAppIds()` only, case/space-insensitive compare, cache with negatives |
| `hooks/wire/PersonaInject.cpp` | new *Friend icon recovery by title* block, before the `ofReal` fallback |
| `core/Settings.{h,cpp}` | `[presence] friend_appid_from_name` key (default `true`, hot-reload) |
| `config/aethercore.example.toml`, `AetherDesk/.../aethercore.toml` | documented key |

The reverse lookup does not enumerate `appinfo.vdf` (100+ MB, versioned binary
format): it uses the Lua-configured library, which by construction holds the
game to recognize in our scenario. Cost: a single scan per title, then cache.

The outbound ShowOnline block stays in tree as a measurement tool but is
**off by default** (`show_online_gameid_type = 0`), and the comment on top
reports the negative outcome so nobody retries it in the future.

### How to verify

On the *watching* PC (no need to play):

1. `[Wire] [DIAG] BUILD showonline-fase1 | ... friend_appid_from_name=1`
2. the friend launches the game with `-aetheronline`
3. wait for: `[GameName] Reverse lookup 'Crusader Kings III' -> app 1158310.`
   and `[Wire.PersonaInject] [DIAG] Friend ...: 480 -> 1158310 by name '...'`
4. the friend list must show the game's icon instead of Spacewar

If (3) appears but no icon, the client has no cached appinfo for the app;
if (3) never appears, the title is not in the local Lua library.

---

## 6. Phase 2 — works; open regression: the Spacewar session drops

User-reported outcome: **the icon appears**. But shortly after, Spacewar closes
and the `-aetheronline` online state is lost.

### Suspect

Both blocks in `PersonaInject` also rewrote **our own entry** (in the logs:
`Patched friend 76561199876393402: 480 -> 1703340`, which is the user
themselves). The Steam client uses its own session state to know which app is
running: if the persona the server sends back for ourselves says "app 1158310"
while the local session manager holds a live 480 session, reconciliation may
conclude the 480 session is stale and close it. With `-aetheronline` the 480
session *is* the online state.

All of that is pointless for the feature: the entry that matters is the one the
**friend** rewrites on **their** PC. Rewriting your own is risk without benefit;
your local view of yourself already belongs to `presenceInjectLocal`.

### Applied correction

Both blocks now skip the entry with `friendid == selfId`. The old `ofReal`
block logs a `[DIAG] self entry arrived as 480 while playing %u; left untouched`
line, telling whether the desync window really exists.

### How to confirm causality

`friend_appid_from_name` hot-reloads: A/B is possible without restarting Steam.

- with `false`, Spacewar does **not** drop -> our rewrite is the cause
- with `false`, Spacewar drops anyway -> pre-existing `-aetheronline` behavior,
  independent of Phase 2

The log needs, around the close moment: `[DIAG] TX: empty games_played`,
`Playing app -> 0`, and the `Masked AppId` / `AetherOnline` lines.

---

## 7. Patch state in tree

| File | Contents |
|---|---|
| `docs/showonline-fase0.patch` | diagnostics: `[DIAG] TX` + `[DIAG] SERVER self-push` |
| `docs/showonline-fase1.patch` | diagnostics **+** CGameID rewrite + settings + TOML |

Additions to the `.proto` (`process_id=9`, `game_flags=11`, `owner_id=12`, verified against SteamKit):
**keep them**, they serve the Family Sharing road.
The `SERVER self-push` log is this feature's measurement tool: keep it until validated,
then degrade it to `AC_LOG_DEBUG_ONCE`.

⚠️ Your installed DLL is **0.9.12**, the repo is at **0.9.10**: apply the patches to your local tree.

---

## 8. Historical note

Pre-Aether history lost (squash `56bccf1`, 19/07/2026). Three references to absent code remain: the
`9aa4a76` commit, the never-imported `docs/03-presence-identity-plan.md` plan, and the **"Meccha"
regression** in `AetherOnlineHooks.cpp:209` — *"leaking real identity into multiplayer routing / friends
presence"*. The "double road" you remembered still exists and is `h_BuildSpawnEnvBlock` (process 480
CGameID for routing, real `SteamOverlayGameId` for DLC and overlay).

---

## 9. `-showonline` as default mode — inventory and design

### 9.1 What `-aetheronline` translates today

Full inventory of the 480↔real points already in tree:

| point | direction | purpose |
|---|---|---|
| `AetherOnlineHooks::h_SpawnProcess` | real → **480** | process-tracking CGameID: what the CM and multiplayer routing see |
| `h_BuildSpawnEnvBlock` | 480 → **real** | overlay CGameID: DLC, depot metadata, overlay identity |
| `CmdUtils::GetAppID` (IPC) | 480 → **real** | a game calling `IClientUtils::GetAppID` gets the true appid |
| `h_GetAppIDForCurrentPipe`, *stats scope* only | 480 → **real** | stats and achievements saved under the right app |
| `OwnershipHooks::h_SendCallbackToPipe` | dual delivery | achievement callbacks to the overlay (real) **and** to the game's handlers (480) |
| `AchievementModule` (wire) | 480 → **real** | `ClientGetUserStatsResponse` |
| `GamesPlayedModule` + `PersonaInject` | annotate | `game_extra_info` / `game_name` with the real title |
| `SteamCapture::CurrentRouteAppId` | real | IPC call routing |

**So: the described idea is already implemented — it is exactly the `-aetheronline`
design.** 480 mask on the wire, translated back locally where needed.

Real hole: **no UGC/Workshop hook exists** (`grep -rn "UGC\|Workshop"` → 0
results). Workshop works only by reflex today, because `GetAppID` and the
overlay CGameID report the true appid. Never verified.

### 9.2 What `-showonline` really adds

Not a new mechanism: **a policy**. Same appid pipeline, less stuff attached.
Concretely it skips `OnlinePayload` (injection into the game process) and
everything only matchmaking needs — weight and risk surface with no reason to
exist for a singleplayer game.

Honest implementation: `kShowOnlineFlag` next to `kAetherOnlineFlag`, entering
the same masking path with a `wantsOnlinePayload` boolean set to false. No
duplicated branches: two parallel paths doing 90% of the same things would
diverge within a month.

### 9.3 The default cannot be "every game"

It must be narrowed, for a reason measured in Phase 0: **a game you truly own
is already broadcast correctly by the CM today**, with icon and name, to *all*
friends, even without Aether. Masking it as 480 would be a net regression.

Correct default:

```
mask as 480  <=>  luadata::HasDepot(app) && !luadata::IsOwned(app)
```

i.e. only Lua-managed, unowned games — the ones the CM silently drops today,
where there is nothing to lose. Plus the exclusion list for real-online titles,
which stay on `-aetheronline`.

### 9.4 Reconstruction when the friend does NOT own the game

Today `ResolveAppIdByName` only searches `luadata::LibraryAppIds()`: when the
friend has no such game configured, no match. Three levels to get past it, in
increasing robustness order.

**(a) Name → appid cache from Steam.** `ISteamApps/GetAppList/v2` returns the
full list (~250k entries, ~10 MB), or `steamcommunity.com/actions/SearchApps/<name>`
for the single query. Saved to disk and refreshed occasionally. Works, but inherits
two weaknesses of name matching: **collisions** (identical titles) and above all
**localization** — the title we ship comes from `gamename::ForApp` in the
*sender's* language, while the receiver's cache is in English.

**(b) The icon for a never-seen appid.** The viewer's client needs the appinfo
(`clienticon`) in cache. But — crucial difference from presence — **PICS metadata
is not license-gated**: the CM answers `ClientPICSProductInfoRequest` for any
appid. The router already handles that message (`AccessToken` module), so the
request can go out. To verify that the client then downloads the icon from the CDN.

**(c) Passing the appid explicitly — the clean road.** Instead of guessing from
the title, ship it: `kClientRichPresenceUpload` (7501) is **already hooked** in
`GamesPlayedModule::HandleRichPresenceUpload`, and in the 23/08 logs it appears
among outbound frames. If a KV like `aether_appid=1158310` survives to the
friends' `PersonaState` (in DIAG the field already reads: `rp_kvs=N`), then the
receiver holds the **exact** appid: no collisions, no language issues, no cache
to maintain. Name matching stays as fallback.

**To measure first:** do rich-presence KVs reach friends when the declared app
is 480? The DIAG `SERVER self-push` already prints `rp_kvs`, so the answer
costs one test round.

---

## 10. Phase 3 — instrumentation for the two open tests

Updated build stamp: look for `[DIAG] BUILD showonline-fase3`.

### Added logs

| where | line | answers |
|---|---|---|
| `PacketRouter::DispatchRecv` | `[DIAG] SharedLibrary msg eMsg=… ` | is the CM ordering the stop, or is the client dying? |
| `PersonaInject` (`ofReal` block) | `[DIAG] self entry arrived as 480 while playing …` | does the suspected desync window exist? |
| `PersonaInject` (self-push) | `[DIAG] SERVER rp kv: 'k' = 'v'` | do our KVs come back from the server? |
| `PersonaInject` (friends) | `[DIAG] FRIEND … rp kv: … (app=…)` | does another account's KV arrive here? |
| `GamesPlayedModule` | `[DIAG] RP upload appid=… 'k' = 'v'` | does the game upload rich presence on its own? |
| `GamesPlayedModule` | `[DIAG] RP probe: sending aether_appid=…` | did the probe launch |
| `PacketRouter::SendClientFrame` | `[DIAG] SendClientFrame eMsg=… -> ok/FAILED` | was the synthetic frame accepted |

### `SendClientFrame` — new tool

`PacketRouter` can now **originate** a client→CM frame: it captures the connection
object and the `CMsgProtoBufHeader` from a real outbound frame, zeroes the job
ids and resends with the eMsg we want. Needed because singleplayers never upload
rich presence on their own: without a probe, `rp_kvs=0` cannot tell "the server
drops them" from "nobody ever sent any".

### Test 1 — does Spacewar still drop? (1 PC)

`friend_appid_from_name` hot-reloads: two runs, `true` and `false`, same game
with `-aetheronline`. In the log, at drop time, look in this order:

1. `[DIAG] SharedLibrary msg` → **present**: the CM orders the stop, our
   rewrite is innocent
2. `[DIAG] TX: empty games_played` with no SharedLibrary → the local client is
   closing the session
3. `[DIAG] self entry arrived as 480 while playing N` → the desync window
   exists; with the correction the entry now stays intact

### Test 2 — does the appid travel in rich presence? (1 PC)

`rp_probe = true`, launch a game with `-aetheronline`, wait ~30 s.

- `[DIAG] RP probe: sending` + `SendClientFrame … -> ok` → probe launched
- `[DIAG] SERVER rp kv: 'aether_appid' = '1703340'` → **round trip complete**:
  the appid can be transmitted, name matching becomes a fallback only
- no `SERVER rp kv` line for ~30 s → the CM drops KVs with app 480, and name
  matching + cache remain

`SendClientFrame … -> FAILED` or `no captured connection` means the probe never
launched: the test is void, not an answer.

### What is NOT measured with one PC

The friend's view. The rebuilt icon and KV propagation to another account need
the second PC — but the icon is already verified, and point 2 answers the
channel question using the server's echo to yourself.
