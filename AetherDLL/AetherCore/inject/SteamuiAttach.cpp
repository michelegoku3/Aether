#include "pch.h"
#include "inject/SteamuiAttach.h"

#include "core/AetherCoreState.h"
#include "inject/ModuleAttach.h"

namespace ac {
namespace {
constexpr const char* kModule = "SteamUI";
} // namespace

bool AttachSteamui(std::uint32_t timeoutMs) {
    if (g_state.steamuiModule)
        return true;

    std::string outcome;
    HMODULE module = WaitForMappedModule("steamui.dll", g_state.steamuiPath, timeoutMs, kModule, outcome);

    g_state.steamuiModule = module;
    {
        std::lock_guard lock(g_state.statusMetadataMutex);
        g_state.steamuiAttach = outcome;
    }
    return module != nullptr;
}

} // namespace ac
