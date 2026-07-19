#include "pch.h"
#include "hooks/ipc/IPCBus.h"

#include <cstring>
#include <unordered_map>

#include "utils/IpcSpec.h"
#include "scripting/LuaData.h"
#include "hooks/ipc/CmdUser.h"
#include "hooks/ipc/CmdUtils.h"
#include "core/Constants.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "utils/PatternEngine.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/ipc/SteamCapture.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "IPC";
using namespace ac::constants;

// steamclient!GetPipeClient(server, hSteamPipe) -> CSteamPipeClient*
using GetPipeClient_t = steam::CSteamPipeClient* (*)(void*, steam::HSteamPipe);
GetPipeClient_t o_GetPipeClient = nullptr;

// steamclient!IPCProcessMessage(server, hSteamPipe, pRead, pWrite) -> bool
using IPCProcessMessage_t = bool (*)(void*, steam::HSteamPipe, steam::CUtlBuffer*, steam::CUtlBuffer*);
IPCProcessMessage_t o_IPCProcessMessage = nullptr;

// Dispatch table keyed by (interfaceId << 32) | funcHash.
// Populated exactly once on the init thread (RegisterIpcBus → CmdXxx::Register)
// before the IPC hook is armed, and read-only afterwards — treated as code,
// not runtime state (admitted module-local per the AetherCoreState.h rule (b)).
std::unordered_map<std::uint64_t, IpcHandlerEntry> s_handlers;

std::uint64_t HandlerKey(std::uint8_t iface, std::uint32_t hash) {
    return (static_cast<std::uint64_t>(iface) << 32) | hash;
}

const IpcHandlerEntry* FindHandler(std::uint8_t iface, std::uint32_t hash) {
    auto it = s_handlers.find(HandlerKey(iface, hash));
    return it != s_handlers.end() ? &it->second : nullptr;
}

steam::CSteamPipeClient* GetPipe(void* server, steam::HSteamPipe hSteamPipe) {
    return o_GetPipeClient ? o_GetPipeClient(server, hSteamPipe) : nullptr;
}

void LogDispatchOnce(const char* handlerName, steam::AppId appId) {
    // Dedup is handled by AC_LOG_DEBUG_ONCE per-session; no local set needed.
    AC_LOG_DEBUG_ONCE(kModule, "Dispatch %s (appId=%u).", handlerName, appId);
}

bool h_IPCProcessMessage(void* server, steam::HSteamPipe hSteamPipe,
                         steam::CUtlBuffer* pRead, steam::CUtlBuffer* pWrite) {
    const IpcHandlerEntry* entry = nullptr;
    steam::CSteamPipeClient* pipe = GetPipe(server, hSteamPipe);

    if (pRead && pRead->TellPut() > 0) {
        const std::uint8_t* data = pRead->Base();
        const std::uint8_t cmd = data[kIpcOffsetCmd];

        if (cmd == ipc_cmd::kHandshake) {
            if (pipe && (pipe->hSteamPipe & 0xFFFF) > kIpcInternalPipeMax) {
                pipewatch::OnHandshake(pipe, pRead);
            }
        } else if (cmd == ipc_cmd::kInterfaceCall) {
            // Steam-internal pipes (low 16 bits <= 2) must pass straight through.
            if (!pipe || (pipe->hSteamPipe & 0xFFFF) <= kIpcInternalPipeMax) {
                return o_IPCProcessMessage(server, hSteamPipe, pRead, pWrite);
            }
            pipewatch::TouchPipe(pipe);
            if (pRead->TellPut() >= kIpcHeaderSize) {
                const std::uint8_t iface = data[kIpcOffsetInterfaceId];
                std::uint32_t funcHash = 0;
                std::memcpy(&funcHash, data + kIpcOffsetFuncHash, sizeof(funcHash));
                entry = FindHandler(iface, funcHash);
            }
        }
    }

    // Always run Steam's original processing first.
    const bool ok = o_IPCProcessMessage(server, hSteamPipe, pRead, pWrite);
    if (!ok || !entry || !pipe) return ok;

    // Only spoof for apps we are configured to handle.
    const steam::AppId appId = pipewatch::AppIdForPipe(pipe);
    if (!luadata::HasDepot(appId)) {
        return ok;
    }

    LogDispatchOnce(entry->name, appId);
    entry->handler(pipe, pRead, pWrite);
    return ok;
}

}  // namespace

void RegisterIpcHandlers(const IpcHandlerEntry* entries, std::size_t count) {
    s_handlers.reserve(s_handlers.size() + count);
    for (std::size_t i = 0; i < count; ++i) {
        // Use per-build spec hash when available, otherwise fall back to the
        // compile-time constant. This keeps IPC dispatch working across Steam
        // client updates that shift internal method hashes.
        std::uint32_t hash = entries[i].funcHash;
        if (auto specHash = ipcspec::ResolveHash(entries[i].name)) {
            if (*specHash != hash) {
                AC_LOG_INFO(kModule, "Spec override: %s 0x%08X -> 0x%08X",
                            entries[i].name, hash, *specHash);
            }
            hash = *specHash;
        }
        s_handlers.emplace(HandlerKey(entries[i].interfaceId, hash), entries[i]);
    }
}

void RegisterIpcBus(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering IPC bus.");

    if (void* addr = pattern::ResolveAddress("GetPipeClient", "steamclient", diversion)) {
        o_GetPipeClient = reinterpret_cast<GetPipeClient_t>(addr);
    } else {
        // Without GetPipeClient we cannot apply the internal-pipe filter safely,
        // so we skip the whole bus rather than risk touching Steam traffic.
        g_state.hookManager.RecordMissed("GetPipeClient");
        AC_LOG_WARN(kModule, "GetPipeClient unresolved; IPC bus disabled.");
        return;
    }

    // Command modules contribute their handler tables before the hook arms.
    CmdUser::Register();
    CmdUtils::Register();

    g_state.hookManager.TryHook("IPCProcessMessage", "steamclient", diversion,
                          o_IPCProcessMessage, h_IPCProcessMessage);
}

}  // namespace ac::hooks
