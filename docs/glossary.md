# Glossary

**What it is:** the vocabulary shared by AetherDesk, AetherDLL and their
documents.

**Read it when:** a term in a document, a log line or a UI label is unclear.

**Not here:** the rules themselves — every entry links to the page that owns
them.

## Steam and the game files

| Term | Meaning |
|---|---|
| **AppID** | Steam application id. A managed game's Lua file is named `<appid>.lua`. |
| **Depot** | A Steam content bucket that belongs to an AppID (base game, DLC, language pack, …). |
| **GID / manifest id** | Decimal identifier of one depot build. Together with the depot it forms the manifest's complete identity. |
| **Manifest** | Encrypted chunk map for one depot at one GID, stored as `<depot>_<gid>.manifest`. |
| **ACF** | Steam's `steamapps/appmanifest_<appid>.acf`: install dir, state flags, build id. |
| **`InstalledDepots`** | ACF block recording, per depot, the manifest GID and size of what is installed. Steam computes every download and update delta from it. |
| **`depotcache`** | `<Steam>/depotcache`, the cache Steam reads manifests from. `<Steam>/config/depotcache` is the secondary cache (read-only for Aether). |
| **`stplug-in`** | `<Steam>/config/stplug-in`, where managed Lua files live. Writing a Lua there is the DLL's hot-reload trigger. |
| **Pin** | `setManifestid(depot, "gid")` in a Lua: locks one depot to one build. |
| **Active pin** | An uncommented pin: the version lock. Automatic pipelines never change it. |
| **Commented pin** | `--setManifestid(...)`: the game is in update mode. It records the last known build and is used when updates are disabled again. |
| **Update mode** | Steam updates enabled for the game, so the pins are commented out. |
| **Build** | A concrete set of pins (one GID per depot) that describes a game version. |

## Aether data

| Term | Meaning |
|---|---|
| **`AetherData`** | Aether's data root: `backup/`, `state/`, `temp/`. |
| **Backup tree** | Per app: original Lua, `history/` of rewritten versions, referenced manifests, achievement snapshots. |
| **Original Lua** | The pristine Lua as received from a source; `history/` keeps every differing rewrite. |
| **Local-first resolution** | Manifest lookup order: `depotcache` → `config\depotcache` → `AetherData` backup → provider. An empty file is never a hit. |
| **Mutating pipeline** | Install, version switch, local import, crack or online deploy: anything that writes a game under the per-game mutation guard. |
| **Contract test** | A test that protects a rule two components must agree on (`ipc_contract_tests.rs`, `manifest_contract_tests.rs`). |

## Sources and quota

| Term | Meaning |
|---|---|
| **Hubcap** | Manifest generation provider, authenticated with a user API key. |
| **Quota** | Shared daily generation budget (1500 game + 500 workshop), reset at EST midnight and enforced by a locked state file. |
| **LuaTools** | Account-based source that returns a package: a Lua plus, when available, its manifests. |
| **Ryuu** | Additional package source. |
| **Package** | A provider response containing a Lua and optionally its `.manifest` files. |
| **Local import** | Installing a game from local archives or folders instead of a provider. |

## UI and online

| Term | Meaning |
|---|---|
| **Library state** | `Available` (Lua only, nothing installed), `Installing` (Steam is writing), `Installed` (Steam holds the content). |
| **Modify popup** | The per-game actions dialog: Enable/Disable Update, Change Version, ONLINE, Remove, Repair manifests, Export Lua and manifests. |
| **Repair manifests** | Restore from backup or regenerate through Hubcap every manifest the game's Lua references. |
| **Export bundle** | "Export Lua and manifests" in the Modify popup: writes the game's Lua and exact pinned manifests as `<game>.zip` in Downloads. |
| **Presence** | Online status Aether reports to Steam friends: `showonline` rewrites presence frames, `aetheronline` also masks the process as Spacewar/480. |
| **UCO2 / OFME** | Third-party online fixes (UCOnline2, online-fix.me) that Aether can detect, enable or clean. |
| **Steamless** | Bundled tool that removes SteamStub DRM from a game executable. |

## Related

- [`docs/shared_contracts.md`](shared_contracts.md) — the rules behind these
  terms.
- [`AetherDesk/docs/ARCHITECTURE.md`](../AetherDesk/docs/ARCHITECTURE.md) and
  [`AetherDLL/docs/ARCHITECTURE.md`](../AetherDLL/docs/ARCHITECTURE.md) — where
  each mechanism lives.
- [`AGENTS.md`](../AGENTS.md) — repository map.
