#include "pch.h"
#include "hooks/ipc/IPCBus.h"

#include <cstring>
#include <string>
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

    if (pRead && pRead->Base() && pRead->TellPut() > 0) {
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

    // Diagnostic (non-blocking): when the IPC spec declares an arg count for
    // this method but the incoming message is too short to carry it, the Steam
    // build's layout may have shifted. Log once per session; dispatch proceeds
    // unchanged so the handler's own bounds checks stay the single gate.
    if (entry->name) {
        if (auto spec = ipcspec::ResolveMethodSpec(entry->name)) {
            if (spec->argc > 0 &&
                pRead->TellPut() < kIpcArgsOffset + static_cast<std::int32_t>(spec->argc) * 4) {
                AC_LOG_WARN_ONCE(kModule, "%s: spec argc=%u but message is short (%d bytes).",
                                 entry->name, spec->argc, pRead->TellPut());
            }
        }
    }

    LogDispatchOnce(entry->name, appId);
    entry->handler(pipe, pRead, pWrite);
    return ok;
}

}  // namespace

void RegisterIpcHandlers(const IpcHandlerEntry* entries, std::size_t count) {
    if (!entries || count == 0) return;
    s_handlers.reserve(s_handlers.size() + count);

    const bool dynamicSpec = ipcspec::IsLoaded();
    if (!dynamicSpec) {
        AC_LOG_INFO_ONCE(kModule, "IPC spec not loaded; using compile-time hashes.");
    }

    for (std::size_t i = 0; i < count; ++i) {
        const IpcHandlerEntry& source = entries[i];
        if (!source.name || !source.handler) {
            AC_LOG_WARN(kModule, "Ignoring IPC handler with incomplete metadata.");
            continue;
        }

        IpcHandlerEntry resolved = source;
        std::uint8_t interfaceId = source.interfaceId;
        std::uint32_t hash = source.funcHash;

        if (dynamicSpec) {
            const std::string name(source.name);
            const std::size_t separator = name.find("::");
            if (separator == std::string::npos || separator == 0) {
                AC_LOG_WARN(kModule, "IPC handler '%s' has invalid qualified name; disabled.",
                            source.name);
                continue;
            }
            const std::string interfaceName = name.substr(0, separator);
            const auto specInterface = ipcspec::ResolveInterfaceId(interfaceName.c_str());
            const auto specHash = ipcspec::ResolveHash(source.name);
            if (!specInterface || !specHash) {
                AC_LOG_WARN(kModule, "IPC handler '%s' disabled: spec metadata missing.",
                            source.name);
                g_state.hookManager.RecordMissed(std::string("IPC:") + source.name);
                continue;
            }
            interfaceId = *specInterface;
            hash = *specHash;
            if (interfaceId != source.interfaceId || hash != source.funcHash) {
                AC_LOG_INFO(kModule, "Spec override: %s iface %u->%u hash 0x%08X->0x%08X",
                            source.name, source.interfaceId, interfaceId,
                            source.funcHash, hash);
            }
        }

        resolved.interfaceId = interfaceId;
        resolved.funcHash = hash;
        const auto [it, inserted] = s_handlers.emplace(HandlerKey(interfaceId, hash), resolved);
        if (!inserted) {
            AC_LOG_WARN(kModule, "IPC handler collision: '%s' conflicts with '%s'; second disabled.",
                        source.name, it->second.name ? it->second.name : "<unnamed>");
            g_state.hookManager.RecordMissed(std::string("IPC collision:") + source.name);
        }
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
