# AetherDesk (Vhierly Fork)

AetherDesk is a Windows desktop tool for browsing the Steam store, managing game
manifests and Lua scripts, and labelling DRM / crack / online-compatibility
status on your library. This fork focuses on **store experience and DRM
detection accuracy**, and pulls selected fixes from upstream
([michelegoku3/Aether](https://github.com/michelegoku3/Aether)).

## What's different from upstream

| Area | This fork | Upstream |
|------|-----------|----------|
| Store pagination | Numbered windowed pages (`1 2 3 4 5 … 12`) with unreachable pages disabled | Prev / Next with `Page X of Y` |
| DRM detection | Steam `drm_notice`, PCGamingWiki fallback, isitcracked crack status | Denuvo detector with a 30-day cache and rate-limit circuit breaker |
| Store filtering | Denuvo / UCO2 / Cracked / DRM-Free filter chips | Search and sort only |
| Library cards | One combined badge (`UCO2 \| Cracked \| Denuvo`) plus Installing state | Per-game badges |
| Family share | Cosmetic family-share labels | Ownership classification and a display-only owner banner |
| Store front | Opens on New Releases (`latest`) | Also `latest` since 1.5.0 |
| Auto-update | AetherDesk from this fork, AetherDLL from upstream | Both from upstream |

## Features

- **Store browsing** — Steam search, genre / sort / storefront filters, badge
  filter chips, numbered paging, trending auto-load, price and discount display
- **DRM detection** — Denuvo Anti-Tamper via Steam `drm_notice`, Steam DRM
  labelling, PCGamingWiki fallback, isitcracked crack status
- **UCO2 compatibility** — labels games supported by UCOnline2 (on-disk
  detection + community list)
- **Library management** — scan Steam libraries, per-game manifest / Lua
  install, badges, pin repair, backup and restore
- **Play from the library** — one click closes memory-hungry non-essential
  processes (Steam, anti-cheat, overlays and running games are whitelisted),
  then hands the game to Steam
- **Steamless integration** — remove Steam DRM from supported titles
- **Online multiplayer** — UCO2 / Online Aether mode with per-game toggle
- **Workshop** — manifest staging and a per-item repair report
- **Logs** — dated log lines and an export bundle (`AetherLog` + `SteamLog`)
- **Settings** — themes (Cyberpunk, Goldmine, Frieren, Halo, plus the
  Wonderland stylesheet), presence injection, Steam diversion mode, HTTP
  allowlist, Hubcap key
- **Auto-update** — in-app update UI; Desk builds update from this fork

## Recent changes

Ported from upstream and adapted to this fork:

- **Close-Steam confirmation** — DLL install / uninstall / reset asks before
  terminating Steam, and running games are closed (and re-swept) first.
- **Store reachability reporting** — a failed search reports when Steam/Hubcap
  never answered (DNS, reset, timeout, TLS) instead of claiming no results.
- **Version bump helper** — `Tools/bump_version.cmd` / `bump_version.ps1`
  validate and dry-run the five Desk version carriers.
- **Store front defaults to New Releases** — opens on `latest`, with a one-time
  migration from the old `upcoming` default.
- **Store front cache warm-up** — the first storefront page is fetched once the
  window is interactive.
- **Dated log lines** — `desk.log` and `main.log` start with
  `YYYY-MM-DD HH:MM:SS.mmm` so the merged view sorts across midnight.
- **Log bundle export** — `AetherLog` and `SteamLog` folders in one ZIP.
- **Read-only online panel** — opening ONLINE no longer rewrites presence
  settings; a change happens only when you pick a mode.
- **Reset confirmation** — Reset Settings asks before wiping every setting.
- **Maintenance badge** — the disabled Auto version tab is marked.
- **Lua + manifest export** — export a game's Lua and pinned manifests as a ZIP.

## Releases

- Stable Desk releases use `desk-v*` tags; test releases use `tdesk-v*` and are
  only offered when **Enable test updates** is on.
- AetherDLL releases stay upstream (`dll-*`); this fork publishes no DLL tags.
- GitHub Actions builds and publishes a portable ZIP on every pushed tag, and
  `workflow_dispatch` can rebuild an existing tag.
- The updater points AetherDesk at this repository by default and AetherDLL at
  `michelegoku3/Aether`; override with `AETHER_RELEASE_OWNER` /
  `AETHER_RELEASE_NAME`.

## How to use AetherDesk

1. Do not keep other Lua tools installed inside Steam.
2. Download the latest release from this fork.
3. Open the app and install AetherDLL from the Aether tab.
4. Scan your Steam library — Denuvo and UCO2 badges appear on game cards.
5. Browse the store, filter by genre / sort / badge, and download manifests.

## Build from source

```bash
cd AetherDesk
npm ci
npm run tauri build
```

Requires Node.js, the Rust stable toolchain and the Visual Studio C++ Build
Tools on Windows.

Or run the full gate script (format, lint, tests, release build, portable ZIP):

```
build_desk.cmd
```

## Repository layout

- `AetherDesk/` — the Tauri desktop app (frontend in `src`, Rust in `src-tauri`)
- `AetherDLL/` — the C++ payload injected into Steam
- `Tools/` — maintenance scripts (version bump, doc checks)
- `docs/` — shared contracts and quality gates
- `build_desk.cmd`, `build_dll.cmd` — full build entry points

## Credits

Original project by [michelegoku3](https://github.com/michelegoku3/Aether).
Fork maintained by [Vhierly](https://github.com/Vhierly).
