#include "pch.h"
#include "hooks/aetheronline/OnlinePayload.h"

#include <cstdint>
#include <string>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "hooks/aetheronline/PresenceSession.h"
#include "utils/Hasher.h"
#include "utils/RemoteInject.h"

#pragma comment(lib, "version.lib")

namespace ac::hooks::onlinepayload {
namespace {

constexpr const char* kModule = "Online.Payload";

// Reads the VS_FIXEDFILEINFO version of a DLL. False when the file has no
// version resource (never in real builds: verify_version.ps1 hard-gates it).
bool GetFileVersion(const std::string& path, DWORD& ms, DWORD& ls) {
    DWORD handle = 0;
    const DWORD size = GetFileVersionInfoSizeA(path.c_str(), &handle);
    if (size == 0)
        return false;
    std::vector<std::uint8_t> data(size);
    if (!GetFileVersionInfoA(path.c_str(), 0, size, data.data()))
        return false;
    void* block = nullptr;
    UINT len = 0;
    if (!VerQueryValueA(data.data(), "\\", &block, &len) || !block || len < sizeof(VS_FIXEDFILEINFO))
        return false;
    const auto* info = static_cast<const VS_FIXEDFILEINFO*>(block);
    if (info->dwSignature != 0xFEEF04BD)
        return false;
    ms = info->dwFileVersionMS;
    ls = info->dwFileVersionLS;
    return true;
}

// The payload and this core are built from the same PROJECT_VERSION (single
// source of truth in the root CMakeLists). A payload whose version differs
// is foreign: stale, planted, or from another release. Refuse it instead of
// loading untrusted code into the game.
bool PayloadVersionMatches() {
    DWORD selfMs = 0, selfLs = 0;
    char selfPath[MAX_PATH] = {};
    if (g_state.selfModule && GetModuleFileNameA(g_state.selfModule, selfPath, MAX_PATH) != 0 &&
        GetFileVersion(selfPath, selfMs, selfLs)) {
        DWORD payMs = 0, payLs = 0;
        if (GetFileVersion(g_state.payloadDllPath, payMs, payLs))
            return payMs == selfMs && payLs == selfLs;
    }
    return false;
}

} // namespace

void MaybeInject(const pipewatch::ProcessSnapshot& snapshot) {
    if (!snapshot.likelyGame || snapshot.steamProcess)
        return;

    const auto active = presence::RealAppId();
    if (active == 0)
        return;

    const auto effectiveAppId = snapshot.appId == constants::kSpacewarAppId ? active : snapshot.appId;
    if (effectiveAppId != active)
        return;

    if (GetFileAttributesA(g_state.payloadDllPath.c_str()) == INVALID_FILE_ATTRIBUTES) {
        // Once per game session (logger dedup) instead of a module-local atomic.
        AC_LOG_WARN_ONCE(kModule, "Payload DLL missing: %s", g_state.payloadDllPath.c_str());
        return;
    }

    {
        std::lock_guard<std::mutex> lock(g_state.onlinePayload.mutex);
        if (g_state.onlinePayload.injectedPids.count(snapshot.pid))
            return;
        g_state.onlinePayload.injectedPids.insert(snapshot.pid);
    }

    // Attestation before touching the target: version pin (same release) plus
    // a SHA-256 line for forensics. Residual TOCTOU (replace between hash
    // and remote LoadLibraryW) is documented; full mitigation would need
    // Authenticode signature verification, out of scope for this batch.
    if (!PayloadVersionMatches()) {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "Payload version mismatch for pid=%u image=%s; refusing injection.", snapshot.pid,
                    snapshot.imageName.c_str());
        return;
    }
    const std::string payloadSha = hasher::ComputeFileSha256(g_state.payloadDllPath);

    // Least privilege: VM_READ was never used and QUERY_INFORMATION is more
    // than IsWow64Process needs (QUERY_LIMITED_INFORMATION suffices).
    HANDLE process =
        OpenProcess(PROCESS_CREATE_THREAD | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_WRITE,
                    FALSE, snapshot.pid);
    if (!process) {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "OpenProcess failed for pid=%u image=%s.", snapshot.pid, snapshot.imageName.c_str());
        return;
    }

    if (inject::IsWow64Target(process)) {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "Skipping x86 target pid=%u image=%s (payload is x64 only).", snapshot.pid,
                    snapshot.imageName.c_str());
        CloseHandle(process);
        return;
    }

    const std::wstring path = strings::Widen(g_state.payloadDllPath);
    const bool ok = !path.empty() && inject::RemoteLoadDll(process, path);
    CloseHandle(process);

    if (ok) {
        ++g_state.onlinePayload.injectSuccessCount;
        AC_LOG_INFO(kModule, "Injected payload into pid=%u image=%s appId=%u sha=%s.", snapshot.pid,
                    snapshot.imageName.c_str(), snapshot.appId, payloadSha.empty() ? "-" : payloadSha.c_str());
    } else {
        ++g_state.onlinePayload.injectFailureCount;
        AC_LOG_WARN(kModule, "Payload injection failed pid=%u image=%s appId=%u.", snapshot.pid,
                    snapshot.imageName.c_str(), snapshot.appId);
    }
}

std::size_t InjectedPidCount() {
    std::lock_guard<std::mutex> lock(g_state.onlinePayload.mutex);
    return g_state.onlinePayload.injectedPids.size();
}

} // namespace ac::hooks::onlinepayload
