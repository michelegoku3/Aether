#pragma once

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// PipeWatch: lightweight per-pipe process identity tracking.
//
// Steam IPC handlers often need to answer "which app does this pipe really
// belong to?". A global answer is fragile for launcher-heavy titles and special
// routes like AetherOnline. PipeWatch observes the IPC handshake, snapshots the
// connecting process (pid, creation time, image, env-derived app id), and lets
// handlers resolve an app id for *that pipe* with graceful fallback.
//
// Scope is intentionally small compared to LumaCore:
//   - no module enumeration
//   - no EOS/Denuvo/child-process side effects
//   - no global hidden state outside AetherCoreState
// ---------------------------------------------------------------------------
namespace ac::pipewatch {

struct ProcessSnapshot {
    std::uint32_t pid = 0;
    std::uint64_t creationTime = 0;
    steam::AppId appId = 0;
    steam::AppId envAppId = 0;
    steam::AppId envSteamAppId = 0;
    steam::AppId envSteamGameId = 0;
    steam::AppId envSteamOverlayGameId = 0;
    std::string appIdSource;  // SteamOverlayGameId | SteamGameId | SteamAppId | fallback
    std::string imagePath;
    std::string imageName;
    bool steamProcess = false;
    bool likelyGame = false;
    bool luaManaged = false;  // script-tracked app, even if genuinely owned
    // Timestamp when this snapshot was captured (A5: used for FIFO eviction
    // when the map exceeds the cap and no dead processes can be reaped).
    std::chrono::steady_clock::time_point capturedAt{};
};

void Reset();
void ResetSessionTracking();
void OnHandshake(steam::CSteamPipeClient* pipe, steam::CUtlBuffer* pRead);
void TouchPipe(steam::CSteamPipeClient* pipe);
std::optional<ProcessSnapshot> SnapshotForPipe(const steam::CSteamPipeClient* pipe);
steam::AppId AppIdForPipe(const steam::CSteamPipeClient* pipe);
std::size_t SnapshotCount();
std::size_t EvictionCount();

}  // namespace ac::pipewatch
