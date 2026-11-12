#include "pch.h"
#include "inject/SteamclientAttach.h"

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "inject/ModuleAttach.h"

namespace ac {
namespace {
constexpr const char* kModule = "Steamclient";
} // namespace

bool AttachSteamclient(std::uint32_t timeoutMs) {
    if (g_state.steamclientModule)
        return true;

    // InitSteamclientStage resolves the path before the first attach attempt;
    // this function never writes shared state besides the attach outcome, so the
    // late-pattern retry can call it while other threads read the path.
    std::string outcome;
    HMODULE module = WaitForMappedModule("steamclient64.dll", g_state.steamclientPath, timeoutMs, kModule, outcome);

    g_state.steamclientModule = module;
    {
        // status.json reads this string under the same lock.
        std::lock_guard lock(g_state.statusMetadataMutex);
        g_state.steamclientAttach = outcome == "mapped"       ? "live-mapped"
                                    : outcome == "not-mapped" ? "live-not-mapped"
                                                              : "path-invalid";
    }
    return module != nullptr;
}

} // namespace ac
