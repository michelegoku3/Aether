#include "pch.h"
#include "hooks/steamui/AppOverviewHooks.h"

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/steamui/AppOverviewPolicy.h"
#include "scripting/LuaData.h"

namespace ac::hooks::steamui {
namespace {

constexpr const char* kModule = "SteamUI.Overview";

// The real signature is forwarded untouched: the detour reads r8 (the app
// object) and passes the same three registers on. It never builds a call with
// a guessed argument count, so extra arguments the client passes stay intact.
using FillInAppOverview_t = void* (*)(void*, void*, steam::AppOverviewPrefix*);
FillInAppOverview_t o_FillInAppOverview = nullptr;

void* h_FillInAppOverview(void* self, void* appOverview, steam::AppOverviewPrefix* app) {
    if (app) {
        g_state.overview.fills.fetch_add(1, std::memory_order_relaxed);

        const auto verdict = policy::InspectOverview(app);
        if (verdict == policy::OverviewVerdict::Ok) {
            bool latchedNow = false;
            {
                std::lock_guard lock(g_state.overview.mutex);
                latchedNow = policy::RecordProbeSample(g_state.overview.probe, *app);
            }
            if (latchedNow) {
                g_state.overview.latched.store(true, std::memory_order_release);
                AC_LOG_INFO(kModule, "AppOverview layout confirmed by two distinct apps.");
            }
        } else {
            g_state.overview.rejects.fetch_add(1, std::memory_order_relaxed);
            {
                std::lock_guard lock(g_state.statusMetadataMutex);
                g_state.overview.lastReject = policy::OverviewVerdictText(verdict);
            }
            // Once per process: a repeated layout fault would flood the log.
            AC_LOG_WARN_ONCE(kModule, "AppOverview rejected: %s", policy::OverviewVerdictText(verdict));
        }

        const auto decision = policy::DecideOwnershipWrite(
            luadata::IsConfigured(app->appId), luadata::IsOwned(app->appId), luadata::IsFamilyShared(app->appId),
            g_state.overview.latched.load(std::memory_order_acquire), static_cast<std::uint32_t>(app->ownershipFlags));
        if (decision.write) {
            app->ownershipFlags = static_cast<steam::AppOwnershipFlags>(
                policy::ApplyOwned(static_cast<std::uint32_t>(app->ownershipFlags)));
            g_state.overview.writes.fetch_add(1, std::memory_order_relaxed);
        }
    }
    return o_FillInAppOverview(self, appOverview, app);
}

} // namespace

bool InstallAppOverviewHooks() {
    if (o_FillInAppOverview)
        return true;
    return g_state.hookManager.TryHook("FillInAppOverview", "steamui", g_state.steamuiModule, o_FillInAppOverview,
                                       h_FillInAppOverview);
}

bool AppOverviewHooksInstalled() { return o_FillInAppOverview != nullptr; }

} // namespace ac::hooks::steamui
