#include "pch.h"
#include "hooks/ipc/PipeWatch.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <cstddef>
#include <cstring>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "utils/EnvReader.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "hooks/onlinefix/OnlinePayload.h"
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

ProcessSnapshot InspectProcess(steam::CSteamPipeClient* pipe) {
    ProcessSnapshot snap{};
    if (!pipe || pipe->clientPid == 0) return snap;

    snap.pid = pipe->clientPid;
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
                                 FALSE, pipe->clientPid);
    if (!process) {
        AC_LOG_DEBUG(kModule, "OpenProcess failed for pid %u.", pipe->clientPid);
        snap.appId = capture::CurrentRouteAppId();
        snap.appIdSource = "fallback";
        snap.imageName = pipe->processName ? pipe->processName : "";
        snap.steamProcess = IsSteamProcessName(snap.imageName);
        snap.likelyGame = !snap.steamProcess && snap.appId != 0;
        snap.luaManaged = snap.appId != 0 && luadata::IsConfigured(snap.appId);
        return snap;
    }

    snap.creationTime = QueryCreationTime(process);
    snap.imagePath = QueryImagePath(process);
    snap.imageName = BaseName(snap.imagePath);
    if (snap.imageName.empty() && pipe->processName) snap.imageName = pipe->processName;

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

void StoreSnapshot(const steam::CSteamPipeClient* pipe, const ProcessSnapshot& snap) {
    if (!pipe) return;
    const std::uint64_t key = EncodePipeKey(pipe);
    if (!key) return;
    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
    g_state.pipeWatch.snapshots[key] = snap;
}

// Tracks the last game appId that triggered a log session reset.
// (State lives in g_state.pipeWatch.lastSessionAppId — centralized state.)


}  // namespace

void Reset() {
    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
    g_state.pipeWatch.snapshots.clear();
}

void OnHandshake(steam::CSteamPipeClient* pipe, steam::CUtlBuffer* pRead) {
    if (!pipe) return;

    if (std::uint32_t pid = ReadHandshakePid(pRead)) {
        pipe->clientPid = pid;
    }
    if (pipe->clientPid == 0) return;

    ProcessSnapshot snap = InspectProcess(pipe);
    StoreSnapshot(pipe, snap);
    hooks::onlinepayload::MaybeInject(snap);
    if (snap.likelyGame) {
        // Only reset dedup sets when a *different* game starts. Child processes
        // of the same session (launcher, game exe, overlay) share the same appId
        // and should not trigger redundant re-emission of ownership/license logs.
        steam::AppId prev = g_state.pipeWatch.lastSessionAppId.load();
        if (snap.appId != prev &&
            g_state.pipeWatch.lastSessionAppId.compare_exchange_strong(prev, snap.appId)) {
            logutil::ResetAllIdLogSessions();
            log::ResetDedup();
        }
    }
    status::Write();
    AC_LOG_INFO(kModule,
                "Handshake pipe=0x%08X pid=%u image=%s appId=%u source=%s env=%u luaManaged=%d.",
                static_cast<std::uint32_t>(pipe->hSteamPipe), snap.pid,
                snap.imageName.empty() ? "-" : snap.imageName.c_str(), snap.appId,
                snap.appIdSource.empty() ? "-" : snap.appIdSource.c_str(), snap.envAppId,
                snap.luaManaged ? 1 : 0);
}

void TouchPipe(steam::CSteamPipeClient* pipe) {
    if (!pipe || pipe->clientPid == 0) return;
    if (SnapshotForPipe(pipe)) return;

    ProcessSnapshot snap = InspectProcess(pipe);
    StoreSnapshot(pipe, snap);
    hooks::onlinepayload::MaybeInject(snap);
    status::Write();
    AC_LOG_DEBUG(kModule,
                 "Late snapshot pipe=0x%08X pid=%u image=%s appId=%u source=%s.",
                 static_cast<std::uint32_t>(pipe->hSteamPipe), snap.pid,
                 snap.imageName.empty() ? "-" : snap.imageName.c_str(), snap.appId,
                 snap.appIdSource.empty() ? "-" : snap.appIdSource.c_str());
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
    if (auto snap = SnapshotForPipe(pipe)) {
        if (snap->appId != 0) return snap->appId;
    }
    return capture::CurrentRouteAppId();
}

std::size_t SnapshotCount() {
    std::lock_guard<std::mutex> lock(g_state.pipeWatch.mutex);
    return g_state.pipeWatch.snapshots.size();
}

void ResetSessionTracking() {
    g_state.pipeWatch.lastSessionAppId.store(0);
}

}  // namespace ac::pipewatch
