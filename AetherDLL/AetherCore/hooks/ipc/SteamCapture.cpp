#include "pch.h"
#include "hooks/ipc/SteamCapture.h"

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/steamclient/OnlineFixHooks.h"
#include "utils/PatternEngine.h"

namespace ac::capture {
namespace {

constexpr const char* kModule = "Capture";

// steamclient!CUtlBuffer::EnsureCapacity(buffer, size) grows the backing store.
using EnsureCapacity_t = void* (*)(steam::CUtlBuffer*, int);
EnsureCapacity_t o_EnsureCapacity = nullptr;

// Depth of IClientUserStats dispatches currently on this thread's stack.
// Thread-local: IPC dispatches run concurrently per pipe on worker threads,
// and the depth must never bleed across threads (see SteamCapture.h).
thread_local std::uint32_t t_statsScopeDepth = 0;

}  // namespace

void EnterStatsScope() {
    ++t_statsScopeDepth;
}

void LeaveStatsScope() {
    if (t_statsScopeDepth > 0) {
        --t_statsScopeDepth;
    } else {
        AC_LOG_WARN(kModule, "LeaveStatsScope called with depth=0; clamping.");
    }
}

bool IsStatsScopeActive() {
    return t_statsScopeDepth > 0;
}

void Init(HMODULE diversion) {
    if (void* addr = pattern::ResolveAddress("CUtlBufferEnsureCapacity", "steamclient", diversion)) {
        o_EnsureCapacity = reinterpret_cast<EnsureCapacity_t>(addr);
        AC_LOG_INFO(kModule, "Resolved CUtlBuffer::EnsureCapacity at 0x%p.", addr);
    } else {
        AC_LOG_WARN(kModule, "CUtlBuffer::EnsureCapacity unresolved; large IPC replies may be capped.");
    }
}

steam::AppId GetAppIdForCurrentPipe() {
    return hooks::CallOriginalGetAppIdForCurrentPipe();
}

steam::AppId CurrentRouteAppId() {
    if (steam::AppId real = g_state.onlineFixRealAppId.load()) return real;
    return GetAppIdForCurrentPipe();
}

void EnsureBufferSize(steam::CUtlBuffer* pWrite, std::int32_t size) {
    if (!pWrite || size <= 0) return;
    // Guard: if EnsureCapacity wasn't resolved, we cannot grow the buffer.
    // Setting pWrite->put without growing would make callers believe the
    // buffer has 'size' bytes available, leading to out-of-bounds writes.
    if (!o_EnsureCapacity) {
        AC_LOG_WARN(kModule, "EnsureCapacity unresolved; cannot grow reply buffer.");
        return;
    }
    o_EnsureCapacity(pWrite, size);
    pWrite->put = size;
}

}  // namespace ac::capture
