#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Thread-safe access layer over the Lua-provided data in AetherCoreState.
//
// Steam worker threads READ this data from hooks while the hot-reload watcher
// thread WRITES it. Every accessor here takes the appropriate lock on
// g_state.lua.mutex, so callers never touch the raw maps and the locking
// discipline lives in one place.
//
// Read accessors take a shared lock; mutators take a unique lock. Reads return
// copies (small: ids, tokens, short hex strings) so no reference can outlive
// the lock.
// ---------------------------------------------------------------------------
namespace ac::luadata {

// ---- Reads (shared lock) ---------------------------------------------------

// True when appId is configured by a script AND not flagged as genuinely owned.
// This is the gate for every spoofing decision.
bool HasDepot(steam::AppId appId);

// True when appId is configured at all (ignores the owned flag).
bool IsConfigured(steam::AppId appId);

// True when CheckAppOwnership has observed a genuine non-family-shared license for appId.
bool IsOwned(steam::AppId appId);

// True when CheckAppOwnership has observed a Steam Family license for appId.
bool IsFamilyShared(steam::AppId appId);

// True when Steam did not provide appId and Aether already answered as owned.
bool IsSpoofed(steam::AppId appId);

// Real family lender captured on the first family answer, or 0.
std::uint32_t FamilyLender(steam::AppId appId);

// Hex depot key for a depot, or nullopt if none/empty.
std::optional<std::string> DepotKeyHex(steam::AppId depotId);

// Access token for an app, or 0 if none.
std::uint64_t AccessToken(steam::AppId appId);

// Manifest override for a depot, or nullopt.
std::optional<ManifestOverride> ManifestOverrideFor(std::uint64_t depotId);

bool HasManifestOverrides();

// Snapshot of every configured depot id (for package-0 injection).
std::vector<steam::AppId> AllDepotIds();

// App id plus every depot contributed by the .lua file(s) that advertise it.
// Used to add one app to package 0 on demand, after Steam classified it.
std::vector<steam::AppId> IdsForApp(steam::AppId appId);

// Every id package 0 must contain for the apps Aether already classified:
// spoofed apps plus family apps with a known lender. Genuinely owned apps and
// apps Steam has not classified yet are deliberately absent.
std::vector<steam::AppId> Package0Ids();

// Snapshot of numeric .lua filename roots that should be advertised to Steam's Library.
std::vector<steam::AppId> LibraryAppIds();

// Map of lua file path -> the app ids that file contributes (depots from
// addappid lines plus the numeric filename root, deduped). Used by the
// ownership unlock summary to report per-file "Unlocked all / Not unlocked".
std::unordered_map<std::string, std::vector<steam::AppId>> ConfiguredIdsByFile();

std::size_t LoadedFileCount();
std::size_t ConfiguredDepotCount();
std::size_t AccessTokenCount();
std::size_t ManifestOverrideCount();
std::string EticketUrl();

// ---- Mutations (unique lock) ----------------------------------------------

// Flags appId as genuinely owned so HasDepot stops spoofing it.
void MarkOwned(steam::AppId appId);

// Flags appId as family-shared so HasDepot stops spoofing it, while cloud can
// stay enabled and family-sharing packet suppression can continue independently.
// `lenderAccountId` is the real owner from AppOwnership.steamId32; 0 means the
// answer did not name one.
void MarkFamilyShared(steam::AppId appId, std::uint32_t lenderAccountId);

// Flags appId as spoofed: Steam did not provide it and Aether owns the answer.
void MarkSpoofed(steam::AppId appId);

// Parse-session bracket. Depots recorded between Begin/End are attributed to
// 'path' for ref-counting. Parsing is single-threaded (init thread, then the
// watcher thread one file at a time), so a single current-file is sufficient.
void BeginFile(const std::string& path);
void EndFile();

// Records a depot under the current file (ref-counted). An empty key never
// overwrites an existing non-empty one. Clears any stale "owned" flag so a
// re-added app re-patches correctly.
void RecordDepot(steam::AppId depotId, const std::string& hexKey);
void RecordLibraryApp(steam::AppId appId);

void SetAccessToken(steam::AppId appId, std::uint64_t token);
void SetManifestOverride(std::uint64_t depotId, ManifestOverride ov);
void SetEticketUrl(const std::string& url);

// Drops a file's depot/token/manifest contributions. Depots whose ref-count
// reaches zero are removed and queued for license removal.
void UnloadFile(const std::string& path);

// Hot-reload hand-off to the license manager.
std::vector<steam::AppId> TakePendingAdditions();
std::vector<steam::AppId> TakePendingRemovals();

// Clears staged add/remove queues. Used after the initial boot scan so startup
// files do not look like hot-reload additions later.
void ClearPendingChanges();

} // namespace ac::luadata
