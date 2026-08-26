#include "pch.h"
#include "hooks/aetheronline/OnlinePayload.h"

#include <string>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "utils/RemoteInject.h"

namespace ac::hooks::onlinepayload {
namespace {

constexpr const char* kModule = "Online.Payload";

}  // namespace

void MaybeInject(const pipewatch::ProcessSnapshot& snapshot) {
    if (!snapshot.likelyGame || snapshot.steamProcess) return;

    const auto active = g_state.aetherOnlineRealAppId.load();
    if (active == 0) return;

    const auto effectiveAppId =
        snapshot.appId == constants::kSpacewarAppId ? active : snapshot.appId;
    if (effectiveAppId != active) return;

    if (GetFileAttributesA(g_state.payloadDllPath.c_str()) == INVALID_FILE_ATTRIBUTES) {
        // Once per game session (logger dedup) instead of a module-local atomic.
        AC_LOG_WARN_ONCE(kModule, "Payload DLL missing: %s", g_state.payloadDllPath.c_str());
        return;
    }

    {
        std::lock_guard<std::mutex> lock(g_state.onlinePayload.mutex);
        if (g_state.onlinePayload.injectedPids.count(snapshot.pid)) return;
        g_state.onlinePayload.injectedPids.insert(snapshot.pid);
    }

    HANDLE process = OpenProcess(PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION |
                                 PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_VM_READ,
                                 FALSE, snapshot.pid);
    if (!process) {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "OpenProcess failed for pid=%u image=%s.",
                    snapshot.pid, snapshot.imageName.c_str());
        return;
    }

    if (inject::IsWow64Target(process)) {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "Skipping x86 target pid=%u image=%s (payload is x64 only).",
                    snapshot.pid, snapshot.imageName.c_str());
        CloseHandle(process);
        return;
    }

    const std::wstring path = inject::Widen(g_state.payloadDllPath);
    const bool ok = !path.empty() && inject::RemoteLoadDll(process, path);
    CloseHandle(process);

    if (ok) {
        ++g_state.onlinePayload.injectSuccessCount;
        AC_LOG_INFO(kModule, "Injected payload into pid=%u image=%s appId=%u.",
                    snapshot.pid, snapshot.imageName.c_str(), snapshot.appId);
    } else {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "Payload injection failed pid=%u image=%s appId=%u.",
                    snapshot.pid, snapshot.imageName.c_str(), snapshot.appId);
    }
}

std::size_t InjectedPidCount() {
    std::lock_guard<std::mutex> lock(g_state.onlinePayload.mutex);
    return g_state.onlinePayload.injectedPids.size();
}

}  // namespace ac::hooks::onlinepayload
