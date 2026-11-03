#include "pch.h"
#include "hooks/steamui/AppOverviewHooks.h"

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/StructGuard.h"
#include "hooks/steamui/AppOverviewPolicy.h"
#include "hooks/steamui/AppRecheckPolicy.h"
#include "scripting/LuaData.h"

namespace ac::hooks::steamui {
namespace {

constexpr const char* kModule = "SteamUI.Overview";

// The client rebuilds the whole dependency plan of an app per recheck, so a full
// queue in one frame would stall the UI: a frame takes a slice and the rest
// follow on the next ones.
constexpr std::size_t kMaxRechecksPerFrame = 8;

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
            {luadata::HasDepot(app->appId), luadata::IsFamilyShared(app->appId), verdict == policy::OverviewVerdict::Ok,
             g_state.overview.latched.load(std::memory_order_acquire), !abi::guard::LayoutContradicted(),
             static_cast<std::uint32_t>(app->ownershipFlags)});
        if (decision.write) {
            app->ownershipFlags = static_cast<steam::AppOwnershipFlags>(
                policy::ApplyOwned(static_cast<std::uint32_t>(app->ownershipFlags)));
            g_state.overview.writes.fetch_add(1, std::memory_order_relaxed);
        }
    }
    return o_FillInAppOverview(self, appOverview, app);
}

// ---- Recheck of a changed app ---------------------------------------------
// The client's own "this app's AppInfo or depot config is stale" entry point.
// Aether never invents an id or a flag: every call carries an id a Lua change
// selected and the client's AppInfoOrConfig bit.

// This detour exists only to learn the object that owns the app list. The first
// `self` seen is kept and the arguments are forwarded unchanged: rewriting them
// would change what the client records, and a hook that edits is no longer a
// seam that can be trusted with an unknown call shape.
using MarkAppChange_t = void* (*)(void*, steam::AppId, std::uint32_t);
MarkAppChange_t o_MarkAppChange = nullptr;

void* h_MarkAppChange(void* self, steam::AppId app, std::uint32_t changeFlags) {
    void* expected = nullptr;
    g_state.overview.appChangeSource.compare_exchange_strong(expected, self, std::memory_order_release,
                                                             std::memory_order_relaxed);
    return o_MarkAppChange(self, app, changeFlags);
}

// The client's per-frame tick: the only thread allowed to touch the app list, so
// the queued ids are handed over here and nowhere else.
using CSteamUIAppControllerRunFrame_t = void* (*)(void*);
CSteamUIAppControllerRunFrame_t o_CSteamUIAppControllerRunFrame = nullptr;

void DrainRechecks(void* source) {
    if (source == nullptr) {
        bool pending = false;
        {
            std::lock_guard lock(g_state.overview.recheckMutex);
            pending = g_state.overview.rechecks.Size() != 0;
        }
        // The ids stay queued: an owner is learned from the first MarkAppChange
        // the client makes on its own, and a later frame drains them then.
        if (pending) {
            AC_LOG_WARN_ONCE(kModule,
                             "App recheck is waiting: MarkAppChange has not run, so no app-list owner is known.");
        }
        return;
    }

    for (std::size_t i = 0; i < kMaxRechecksPerFrame; ++i) {
        steam::AppId app = 0;
        {
            // Only the pop is guarded. The client call below may take its own
            // locks and must never wait for the worker that is pushing.
            std::lock_guard lock(g_state.overview.recheckMutex);
            if (!g_state.overview.rechecks.Pop(app))
                break;
        }
        o_MarkAppChange(source, app, steam::kAppChangeAppInfoOrConfig);
        g_state.overview.recheckCalls.fetch_add(1, std::memory_order_relaxed);
        AC_LOG_INFO(kModule, "Asked the client to re-evaluate app %u after its manifest pins changed.", app);
    }
}

void* h_CSteamUIAppControllerRunFrame(void* self) {
    // Forward first: the client's frame work keeps the order it would have had
    // without us, and the drain is extra work on a frame that already ran.
    void* result = o_CSteamUIAppControllerRunFrame(self);
    DrainRechecks(g_state.overview.appChangeSource.load(std::memory_order_acquire));
    return result;
}

} // namespace

bool InstallAppOverviewHooks() {
    if (o_FillInAppOverview)
        return true;
    // The batch that calls this latches on the card hook alone, so a miss here
    // returns early and the whole group is attempted again on its next run.
    if (!g_state.hookManager.TryHook("FillInAppOverview", "steamui", g_state.steamuiModule, o_FillInAppOverview,
                                     h_FillInAppOverview)) {
        return false;
    }
    // Each recheck pattern is optional on its own: a table that lacks one leaves
    // that half of the pair off and `recheck_state` reports `unsupported`, while
    // the card write above keeps working. RecordMissed dedups by name, so the
    // attach worker's retries cannot grow the miss report.
    g_state.hookManager.TryHook("MarkAppChange", "steamui", g_state.steamuiModule, o_MarkAppChange, h_MarkAppChange);
    g_state.hookManager.TryHook("CSteamUIAppControllerRunFrame", "steamui", g_state.steamuiModule,
                                o_CSteamUIAppControllerRunFrame, h_CSteamUIAppControllerRunFrame);
    return true;
}

bool AppOverviewHooksInstalled() { return o_FillInAppOverview != nullptr; }

void RequestAppRecheck(const std::vector<steam::AppId>& ids) {
    std::size_t refused = 0;
    {
        std::lock_guard lock(g_state.overview.recheckMutex);
        for (const steam::AppId app : ids) {
            // Push answers false for two different outcomes: a full queue drops
            // the request, an id already queued is still going to be rechecked.
            // Only the first loses the app, and the capacity check is the only
            // way to tell them apart.
            if (g_state.overview.rechecks.Size() >= policy::RecheckQueue::kCapacity) {
                ++refused;
                continue;
            }
            g_state.overview.rechecks.Push(app);
        }
    }
    if (refused != 0) {
        AC_LOG_WARN_ONCE(kModule, "App recheck queue full: %zu request(s) dropped; the next apply re-offers them.",
                         refused);
    }
}

bool AppRecheckSupported() { return o_MarkAppChange != nullptr && o_CSteamUIAppControllerRunFrame != nullptr; }

} // namespace ac::hooks::steamui
