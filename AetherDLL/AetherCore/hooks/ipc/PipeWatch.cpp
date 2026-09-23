#include "pch.h"
#include "hooks/ipc/PipeWatch.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <chrono>
#include <cstddef>
#include <cstring>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>

#include "core/AetherCoreState.h"
#include "core/Workers.h"
#include "core/Constants.h"
#include "utils/EnvReader.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "hooks/aetheronline/OnlinePayload.h"
#include "diagnostics/StatusWriter.h"
#include "hooks/ipc/SteamCapture.h"
#include "utils/SmartIdLog.h"

namespace ac::pipewatch {
namespace {

constexpr const char* kModule = "PipeWatch";

std::string LowerAscii(std::string_view text) {
    std::string out(text);
    std::transform(out.begin(), out.end(), out.begin(), [](unsigned char ch) {
        return static_cast<char>(std::tolower(ch));
    });
    return out;
}

std::string BaseName(std::string_view path) {
    const std::size_t slash = path.find_last_of("\\/");
    if (slash == std::string_view::npos) return std::string(path);
    return std::string(path.substr(slash + 1));
}

bool IsSteamProcessName(std::string_view imageName) {
    static constexpr std::array<std::string_view, 6> kSteamNames = {
        "steam.exe",
        "steamwebhelper.exe",
        "steamservice.exe",
        "steamerrorreporter.exe",
        "gameoverlayui.exe",
        "gameoverlayui64.exe",
    };
    const std::string lowered = LowerAscii(imageName);
    return std::find(kSteamNames.begin(), kSteamNames.end(), lowered) != kSteamNames.end();
}

std::uint64_t EncodePipeKey(const steam::CSteamPipeClient* pipe) {
    if (!pipe) return 0;
    return (static_cast<std::uint64_t>(pipe->clientPid) << 32) |
           static_cast<std::uint32_t>(pipe->hSteamPipe);
}

std::uint64_t QueryCreationTime(HANDLE process) {
    FILETIME created{}, exited{}, kernel{}, user{};
    if (!GetProcessTimes(process, &created, &exited, &kernel, &user)) return 0;
    return (static_cast<std::uint64_t>(created.dwHighDateTime) << 32) |
           static_cast<std::uint64_t>(created.dwLowDateTime);
}

std::string QueryImagePath(HANDLE process) {
    char path[MAX_PATH] = {};
    DWORD size = MAX_PATH;
    if (!QueryFullProcessImageNameA(process, 0, path, &size) || size == 0) return {};
    return std::string(path, size);
}

// Ispezione per (pid, nome processo): nessun puntatore "vivo" di Steam
// attraversa i thread, quindi può girare sul task queue.
ProcessSnapshot InspectProcess(std::uint32_t pid, const std::string& processName) {
    ProcessSnapshot snap{};
    if (pid == 0) return snap;

    snap.pid = pid;
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
                                 FALSE, pid);
    if (!process) {
        AC_LOG_DEBUG(kModule, "OpenProcess failed for pid %u.", pid);
        snap.appId = capture::CurrentRouteAppId();
        snap.appIdSource = "fallback";
        snap.imageName = processName;
        snap.steamProcess = IsSteamProcessName(snap.imageName);
        snap.likelyGame = !snap.steamProcess && snap.appId != 0;
        snap.luaManaged = snap.appId != 0 && luadata::IsConfigured(snap.appId);
        return snap;
    }

    snap.creationTime = QueryCreationTime(process);
    snap.imagePath = QueryImagePath(process);
    snap.imageName = BaseName(snap.imagePath);
    if (snap.imageName.empty()) snap.imageName = processName;

    // Environment-block AppId resolution delegated to EnvReader.
    if (auto ids = env::ReadSteamEnvAppIds(process)) {
        snap.envAppId = ids->selected;
        snap.envSteamAppId = ids->steamAppId;
        snap.envSteamGameId = ids->steamGameId;
        snap.envSteamOverlayGameId = ids->steamOverlayGameId;
        snap.appId = ids->selected;
        snap.appIdSource = ids->source;
    } else {
        snap.appId = capture::CurrentRouteAppId();
        snap.appIdSource = "fallback";
    }

    CloseHandle(process);

    snap.steamProcess = IsSteamProcessName(snap.imageName);
    snap.likelyGame = !snap.steamProcess && snap.appId != 0;
    snap.luaManaged = snap.appId != 0 && luadata::IsConfigured(snap.appId);
    return snap;
}

std::uint32_t ReadHandshakePid(steam::CUtlBuffer* pRead) {
    if (!pRead || pRead->TellPut() < constants::kIpcHandshakeMinSize) return 0;
    const std::uint8_t* raw = pRead->Base();
    std::uint32_t pid = 0;
    std::memcpy(&pid, raw + constants::kIpcHandshakePidOffset, sizeof(pid));
    return pid;
}

// ---------------------------------------------------------------------------
// Eviction helpers (A5). Caller must hold g_state.pipeWatch.mutex.
//
// Two-level strategy:
//   1. EvictDeadLocked — probe each non-Steam entry with OpenProcess; if the
//      process is gone, drop the snapshot. This is the most precise method
//      (removes only truly dead entries) but costs one syscall per candidate.
//   2. EvictOldestLocked — fallback FIFO by capturedAt when dead-process
//      reaping alone was not enough to get below the cap.
//
// Eviction is opportunistic: it runs only when the map has reached the cap,
// so normal sessions (< 64 pipes) never pay the cost.
// ---------------------------------------------------------------------------

bool IsProcessAlive(std::uint32_t pid) {
    if (pid == 0) return false;
    HANDLE h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!h) return false;
    DWORD exitCode = 0;
    const bool alive = GetExitCodeProcess(h, &exitCode) && exitCode == STILL_ACTIVE;
    CloseHandle(h);
    return alive;
}

// Removes snapshots whose process is no longer running. Steam processes are
// never evicted (they persist for the entire session and probing them is
// wasteful). Returns the number of entries removed.
std::size_t EvictDeadLocked() {
    std::size_t removed = 0;
    for (auto it = g_state.pipeWatch.snapshots.begin();
         it != g_state.pipeWatch.snapshots.end();) {
        const auto& snap = it->second;
        // Never evict Steam's own processes: they are always alive and
        // re-probing them on every eviction cycle is pure overhead.
        if (snap.steamProcess) {
            ++it;
            continue;
        }
        if (!IsProcessAlive(snap.pid)) {
            AC_LOG_DEBUG(kModule, "Evicting dead snapshot pid=%u image=%s appId=%u.",
                         snap.pid, snap.imageName.empty() ? "-" : snap.imageName.c_str(),
                         snap.appId);
            it = g_state.pipeWatch.snapshots.erase(it);
            ++removed;
        } else {
            ++it;
        }
    }
    return removed;
}

// Removes the single oldest snapshot (by capturedAt) to make room. Skips
// Steam processes. Returns true if an entry was removed.
bool EvictOldestLocked() {
    using Iter = decltype(g_state.pipeWatch.snapshots)::iterator;
    Iter oldest = g_state.pipeWatch.snapshots.end();

    for (auto it = g_state.pipeWatch.snapshots.begin();
         it != g_state.pipeWatch.snapshots.end(); ++it) {
        if (it->second.steamProcess) continue;
        if (oldest == g_state.pipeWatch.snapshots.end() ||
            it->second.capturedAt < oldest->second.capturedAt) {
            oldest = it;
        }
    }

    if (oldest == g_state.pipeWatch.snapshots.end()) return false;

    AC_LOG_DEBUG(kModule, "Evicting oldest snapshot pid=%u image=%s appId=%u (cap=%zu).",
                 oldest->second.pid,
                 oldest->second.imageName.empty() ? "-" : oldest->second.imageName.c_str(),
                 oldest->second.appId,
                 constants::kPipeWatchMaxSnapshots);
    g_state.pipeWatch.snapshots.erase(oldest);
    return true;
}

// Orchestrator: called before inserting a new snapshot when the map is at
// capacity. First reaps dead processes; if that is not enough, drops the
// oldest entry one at a time until there is room.
void EvictIfNeededLocked() {
    if (g_state.pipeWatch.snapshots.size() < constants::kPipeWatchMaxSnapshots) return;

    // Phase 1: reap dead processes (most precise).
    const std::size_t deadRemoved = EvictDeadLocked();
    if (deadRemoved > 0) {
        g_state.pipeWatch.evictionCount.fetch_add(deadRemoved, std::memory_order_relaxed);
    }
    if (g_state.pipeWatch.snapshots.size() < constants::kPipeWatchMaxSnapshots) return;

    // Phase 2: FIFO fallback — drop oldest until under cap.
    std::size_t fifoRemoved = 0;
    while (g_state.pipeWatch.snapshots.size() >= constants::kPipeWatchMaxSnapshots) {
        if (!EvictOldestLocked()) break;  // nothing left to evict (all Steam)
        ++fifoRemoved;
    }
    if (fifoRemoved > 0) {
        g_state.pipeWatch.evictionCount.fetch_add(fifoRemoved, std::memory_order_relaxed);
    }
}

void StoreSnapshotByKey(std::uint64_t key, ProcessSnapshot snap) {
    if (!key) return;

    // Stamp the capture time for FIFO eviction ordering (A5).
    snap.capturedAt = std::chrono::steady_clock::now();

    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);

    // If this key already exists, it is an update (TouchPipe re-snapshot) —
    // no eviction needed since the count does not grow.
    const bool isUpdate = g_state.pipeWatch.snapshots.count(key) > 0;
    if (!isUpdate) {
        EvictIfNeededLocked();
    }

    g_state.pipeWatch.snapshots[key] = std::move(snap);
}

// Tracks the last game appId that triggered a log session reset.
// (State lives in g_state.pipeWatch.lastSessionAppId — centralized state.)


}  // namespace

void Reset() {
    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
    g_state.pipeWatch.snapshots.clear();
}

// Il lavoro pesante dell'handshake (OpenProcess, lettura env remota, immagine,
// eventuale iniezione payload con attesa fino a 5 s) NON gira più sul thread
// IPC di Steam: viene eseguito sul task queue. Sul thread IPC restano solo la
// lettura del pid e l'accodamento (O(1)).
void CompleteHandshakeAsync(std::uint64_t key, std::uint32_t hSteamPipe,
                            std::uint32_t pid, std::string processName) {
    ProcessSnapshot snap = InspectProcess(pid, processName);
    StoreSnapshotByKey(key, std::move(snap));
    // Re-read the stored snapshot (same source of truth every consumer uses).
    const auto stored = [&]() -> std::optional<ProcessSnapshot> {
        std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
        auto it = g_state.pipeWatch.snapshots.find(key);
        if (it == g_state.pipeWatch.snapshots.end()) return std::nullopt;
        return it->second;
    }();
    if (!stored) return;
    hooks::onlinepayload::MaybeInject(*stored);
    if (stored->likelyGame) {
        // Only reset dedup sets when a *different* game starts. Child processes
        // of the same session (launcher, game exe, overlay) share the same appId
        // and should not trigger redundant re-emission of ownership/license logs.
        steam::AppId prev = g_state.pipeWatch.lastSessionAppId.load();
        if (stored->appId != prev &&
            g_state.pipeWatch.lastSessionAppId.compare_exchange_strong(prev, stored->appId)) {
            logutil::ResetAllIdLogSessions();
            log::ResetDedup();
        }
    }
    status::Write();
    AC_LOG_INFO(kModule,
                "Handshake pipe=0x%08X pid=%u image=%s appId=%u source=%s env=%u luaManaged=%d (async).",
                hSteamPipe, stored->pid,
                stored->imageName.empty() ? "-" : stored->imageName.c_str(), stored->appId,
                stored->appIdSource.empty() ? "-" : stored->appIdSource.c_str(), stored->envAppId,
                stored->luaManaged ? 1 : 0);
}

void OnHandshake(steam::CSteamPipeClient* pipe, steam::CUtlBuffer* pRead) {
    if (!pipe) return;

    if (std::uint32_t pid = ReadHandshakePid(pRead)) {
        pipe->clientPid = pid;
    }
    if (pipe->clientPid == 0) return;

    const std::uint64_t key = EncodePipeKey(pipe);
    // Copia per valore: il puntatore di Steam e il suo processName non sono
    // sicuri oltre questa chiamata.
    std::string processName = pipe->processName ? pipe->processName : "";
    const std::uint32_t pipeHandle = static_cast<std::uint32_t>(pipe->hSteamPipe);
    const std::uint32_t pidCopy = pipe->clientPid;
    if (!workers::Submit([key, pipeHandle, pidCopy, processName = std::move(processName)] {
            CompleteHandshakeAsync(key, pipeHandle, pidCopy, processName);
        })) {
        AC_LOG_DEBUG(kModule, "Handshake inspection for pid %u skipped (workers shut down).", pidCopy);
    }
}

void TouchPipe(steam::CSteamPipeClient* pipe) {
    if (!pipe || pipe->clientPid == 0) return;
    if (SnapshotForPipe(pipe)) return;

    const std::uint64_t key = EncodePipeKey(pipe);
    std::string processName = pipe->processName ? pipe->processName : "";
    const std::uint32_t pipeHandle = static_cast<std::uint32_t>(pipe->hSteamPipe);
    const std::uint32_t pidCopy = pipe->clientPid;
    if (!workers::Submit([key, pipeHandle, pidCopy, processName = std::move(processName)] {
            CompleteHandshakeAsync(key, pipeHandle, pidCopy, processName);
        })) {
        AC_LOG_DEBUG(kModule, "Late snapshot for pid %u skipped (workers shut down).", pidCopy);
    }
}

std::optional<ProcessSnapshot> SnapshotForPipe(const steam::CSteamPipeClient* pipe) {
    if (!pipe || pipe->clientPid == 0) return std::nullopt;
    const std::uint64_t key = EncodePipeKey(pipe);
    if (!key) return std::nullopt;

    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
    auto it = g_state.pipeWatch.snapshots.find(key);
    if (it == g_state.pipeWatch.snapshots.end()) return std::nullopt;
    return it->second;
}

steam::AppId AppIdForPipe(const steam::CSteamPipeClient* pipe) {
    // Lettura diretta del solo appId sotto lock: questo gira su OGNI messaggio
    // IPC, quindi niente copia dello snapshot (stringhe incluse).
    if (pipe && pipe->clientPid != 0) {
        const std::uint64_t key = EncodePipeKey(pipe);
        std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
        auto it = g_state.pipeWatch.snapshots.find(key);
        if (it != g_state.pipeWatch.snapshots.end() && it->second.appId != 0) {
            return it->second.appId;
        }
    }
    return capture::CurrentRouteAppId();
}

std::size_t SnapshotCount() {
    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
    return g_state.pipeWatch.snapshots.size();
}

std::size_t EvictionCount() {
    return g_state.pipeWatch.evictionCount.load(std::memory_order_relaxed);
}

void ResetSessionTracking() {
    g_state.pipeWatch.lastSessionAppId.store(0);
}

}  // namespace ac::pipewatch
