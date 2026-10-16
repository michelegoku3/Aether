#include "pch.h"
#include "hooks/ipc/IPCBus.h"

#include <atomic>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>

#include "core/Constants.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "hooks/ipc/CmdUser.h"
#include "hooks/ipc/CmdUtils.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/ipc/SteamCapture.h"
#include "scripting/LuaData.h"
#include "utils/IpcSpec.h"
#include "utils/PatternEngine.h"

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

// Dispatch table keyed by (interfaceId << 32) | funcHash, published via
// atomic shared_ptr swap: readers on Steam IPC threads load once (acquire)
// and search without any lock, so a late re-registration can never race
// dispatch. Writers (init thread / late-pattern retry, already serialized
// by the bootstrap batch mutex) take the writer mutex as belt and braces.
// Still module-local code-dispatch metadata, not runtime state (admitted
// module-local per the AetherCoreState.h rule (b)).
using HandlerMap = std::unordered_map<std::uint64_t, IpcHandlerEntry>;
std::atomic<std::shared_ptr<const HandlerMap>> s_handlers{std::make_shared<const HandlerMap>()};
std::mutex s_handlersMutex; // writers only; readers never touch it
// Set when a registration batch ran without the spec: those entries keep
// compile-time hashes. Used only to scope the stale-spec warning below.
std::atomic<bool> s_registeredWithoutSpec{false};

std::uint64_t HandlerKey(std::uint8_t iface, std::uint32_t hash) {
    return (static_cast<std::uint64_t>(iface) << 32) | hash;
}

// Looks up one entry in an already-held snapshot. The caller must keep the
// shared_ptr alive while using the returned pointer: a late re-registration
// swaps s_handlers, and the previous map is destroyed once its last snapshot
// goes away. Returning a raw pointer without the snapshot would be
// use-after-free across that swap.
const IpcHandlerEntry* FindInSnapshot(const HandlerMap& map, std::uint8_t iface, std::uint32_t hash) {
    const auto it = map.find(HandlerKey(iface, hash));
    return it != map.end() ? &it->second : nullptr;
}

steam::CSteamPipeClient* GetPipe(void* server, steam::HSteamPipe hSteamPipe) {
    return o_GetPipeClient ? o_GetPipeClient(server, hSteamPipe) : nullptr;
}

void LogDispatchOnce(const char* handlerName, steam::AppId appId) {
    // Dedup is handled by AC_LOG_DEBUG_ONCE per-session; no local set needed.
    AC_LOG_DEBUG_ONCE(kModule, "Dispatch %s (appId=%u).", handlerName, appId);
}

// RAII: brackets the duration of an IClientUserStats IPC dispatch so
// GetAppIDForCurrentPipe resolves the AetherOnline Spacewar/480 masquerade to the
// real app id (see capture::EnterStatsScope / AetherOnlineHooks). Only activates
// for stats interface calls; no-ops otherwise.
struct StatsScopeGuard {
    explicit StatsScopeGuard(bool active) : m_active(active) {
        if (m_active)
            capture::EnterStatsScope();
    }
    ~StatsScopeGuard() {
        if (m_active)
            capture::LeaveStatsScope();
    }

  private:
    bool m_active;
};

bool h_IPCProcessMessage(void* server, steam::HSteamPipe hSteamPipe, steam::CUtlBuffer* pRead,
                         steam::CUtlBuffer* pWrite) {
    const IpcHandlerEntry* entry = nullptr;
    // Owns the dispatch-table snapshot entry points into: a concurrent
    // RegisterIpcHandlers swap must not destroy the map mid-dispatch.
    std::shared_ptr<const HandlerMap> handlerMap;
    steam::CSteamPipeClient* pipe = GetPipe(server, hSteamPipe);
    bool statsScope = false;

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
                // Stats calls are bracketed so the client resolves the real app
                // id during Steam's own stats processing (see StatsScopeGuard).
                statsScope = (iface == ipc_iface::kClientUserStats);
                std::uint32_t funcHash = 0;
                std::memcpy(&funcHash, data + kIpcOffsetFuncHash, sizeof(funcHash));
                handlerMap = s_handlers.load(std::memory_order_acquire);
                entry = FindInSnapshot(*handlerMap, iface, funcHash);
            }
        }
    }

    // Always run Steam's original processing first.
    const StatsScopeGuard statsGuard(statsScope);
    const bool ok = o_IPCProcessMessage(server, hSteamPipe, pRead, pWrite);
    if (!ok || !entry || !pipe)
        return ok;

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
            if (spec->argc > 0 && pRead->TellPut() < kIpcArgsOffset + static_cast<std::int32_t>(spec->argc) * 4) {
                AC_LOG_WARN_ONCE(kModule, "%s: spec argc=%u but message is short (%d bytes).", entry->name, spec->argc,
                                 pRead->TellPut());
            }
        }
    }

    LogDispatchOnce(entry->name, appId);
    entry->handler(pipe, pRead, pWrite);
    return ok;
}

} // namespace

void RegisterIpcHandlers(const IpcHandlerEntry* entries, std::size_t count) {
    if (!entries || count == 0)
        return;
    // Build the merged table locally, then publish with one atomic swap:
    // in-flight dispatches keep reading the previous immutable snapshot.
    std::lock_guard<std::mutex> wlock(s_handlersMutex);
    auto merged = std::make_shared<HandlerMap>(*s_handlers.load(std::memory_order_acquire));
    merged->reserve(merged->size() + count);

    const bool dynamicSpec = ipcspec::IsLoaded();
    if (!dynamicSpec) {
        AC_LOG_INFO_ONCE(kModule, "IPC spec not loaded; using compile-time hashes.");
        s_registeredWithoutSpec.store(true, std::memory_order_release);
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
                AC_LOG_WARN(kModule, "IPC handler '%s' has invalid qualified name; disabled.", source.name);
                continue;
            }
            const std::string interfaceName = name.substr(0, separator);
            const auto specInterface = ipcspec::ResolveInterfaceId(interfaceName.c_str());
            const auto specHash = ipcspec::ResolveHash(source.name);
            if (!specInterface || !specHash) {
                AC_LOG_WARN(kModule, "IPC handler '%s' disabled: spec metadata missing.", source.name);
                g_state.hookManager.RecordMissed(std::string("IPC:") + source.name, MissReason::MetadataUnavailable);
                continue;
            }
            interfaceId = *specInterface;
            hash = *specHash;
            if (interfaceId != source.interfaceId || hash != source.funcHash) {
                AC_LOG_INFO(kModule, "Spec override: %s iface %u->%u hash 0x%08X->0x%08X", source.name,
                            source.interfaceId, interfaceId, source.funcHash, hash);
            }
        }

        resolved.interfaceId = interfaceId;
        resolved.funcHash = hash;
        const auto [it, inserted] = merged->emplace(HandlerKey(interfaceId, hash), resolved);
        if (!inserted) {
            AC_LOG_WARN(kModule, "IPC handler collision: '%s' conflicts with '%s'; second disabled.", source.name,
                        it->second.name ? it->second.name : "<unnamed>");
            g_state.hookManager.RecordMissed(std::string("IPC collision:") + source.name, MissReason::HandlerCollision,
                                             it->second.name ? it->second.name : "<unnamed>");
        }
    }
    s_handlers.store(std::move(merged), std::memory_order_release);
}

void RegisterIpcBus(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    // Idempotent re-run (late-pattern retry re-runs the whole client batch):
    // an armed bus must not re-register its command handler tables.
    if (o_GetPipeClient) {
        // Visibility for the stale-spec gap: tables registered before the
        // spec arrived keep compile-time hashes until the next Steam restart.
        // A late re-registration with new hashes is a follow-up (it needs a
        // re-resolve of every entry against the loaded spec).
        if (s_registeredWithoutSpec.load(std::memory_order_acquire) && ipcspec::IsLoaded())
            AC_LOG_WARN_ONCE(kModule, "IPC spec arrived after arming; compile-time hashes stay active.");
        AC_LOG_DEBUG(kModule, "IPC bus already armed; skipping re-registration.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering IPC bus.");

    if (void* addr = pattern::ResolveAddress("GetPipeClient", "steamclient", diversion)) {
        o_GetPipeClient = reinterpret_cast<GetPipeClient_t>(addr);
    } else {
        // Without GetPipeClient we cannot apply the internal-pipe filter safely,
        // so we skip the whole bus rather than risk touching Steam traffic.
        // The caller log states the consequence (bus disabled), which the
        // generic miss line cannot; the reason itself comes from RecordMissed.
        g_state.hookManager.RecordMissed("GetPipeClient", MissReason::PatternUnresolved);
        return;
    }

    // Command modules contribute their handler tables before the hook arms.
    CmdUser::Register();
    CmdUtils::Register();

    g_state.hookManager.TryHook("IPCProcessMessage", "steamclient", diversion, o_IPCProcessMessage,
                                h_IPCProcessMessage);
}

} // namespace ac::hooks
