#include "pch.h"
#include "hooks/steamui/AppOverviewHooks.h"

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/StructGuard.h"
#include "diagnostics/StatusWriter.h"
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

// `no-source` is an answer the status file has to show, but the frame runs every
// tick: publishing it once is enough, and repeating the request would be churn.
std::atomic<bool> g_noSourceStatusWritten{false};

// The real signature is forwarded untouched: the detour reads r8 (the app
// object) and passes the same three registers on. It never builds a call with
// a guessed argument count, so extra arguments the client passes stay intact.
using FillInAppOverview_t = void* (*)(void*, void*, steam::AppOverviewPrefix*);
FillInAppOverview_t o_FillInAppOverview = nullptr;

// The tracking sets, read in one lock. `full` is the capacity answer for a
// management that is not yet recorded: a card Aether cannot remember would leave
// an unclearable bit, so that write is refused instead.
struct Tracking {
    bool retired = false;
    bool tracked = false;
    bool full = false;
};

Tracking ReadTracking(steam::AppId app) {
    std::lock_guard lock(g_state.overview.stampMutex);
    const bool tracked = g_state.overview.tracked.count(app) != 0;
    return {g_state.overview.retired.count(app) != 0, tracked,
            !tracked && g_state.overview.tracked.size() >= policy::kMaxTrackedApps};
}

// Every managed card is recorded, whether or not this fill had to write the bit:
// the client often sets it itself from package 0, and that bit becomes Aether's
// to take back the moment the `.lua` goes away.
void TrackManaged(steam::AppId app) {
    std::lock_guard lock(g_state.overview.stampMutex);
    g_state.overview.tracked.insert(app);
    // A managed card is not retired any more: the file either came back or this
    // is the first fill after a late hook install. It is not hidden either: the
    // client built it again, which is what the hidden set was waiting for.
    g_state.overview.retired.erase(app);
    g_state.overview.hidden.erase(app);
}

// The bit was taken back, so the client drops the app. The id is kept: a later
// `.lua` for the same app has to be announced as a new app, because the client
// no longer holds an object that a config change could rebuild.
void ForgetTracking(steam::AppId app) {
    bool refused = false;
    {
        std::lock_guard lock(g_state.overview.stampMutex);
        g_state.overview.tracked.erase(app);
        g_state.overview.retired.erase(app);
        if (g_state.overview.hidden.size() >= policy::kMaxTrackedApps) {
            refused = g_state.overview.hidden.count(app) == 0;
        } else {
            g_state.overview.hidden.insert(app);
        }
    }
    if (refused) {
        AC_LOG_WARN_ONCE(kModule,
                         "Hidden set full: app %u not recorded; re-adding its .lua needs a restart to show it.", app);
    }
}

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

        policy::WriteInputs inputs;
        inputs.managed = luadata::HasDepot(app->appId);
        inputs.genuinelyOwned = luadata::IsOwned(app->appId);
        inputs.familyShared = luadata::IsFamilyShared(app->appId);
        const Tracking tracking = ReadTracking(app->appId);
        inputs.retired = tracking.retired;
        inputs.tracked = tracking.tracked;
        inputs.trackingFull = tracking.full;
        inputs.sampleOk = verdict == policy::OverviewVerdict::Ok;
        inputs.probeLatched = g_state.overview.latched.load(std::memory_order_acquire);
        inputs.layoutTrusted = !abi::guard::LayoutContradicted();
        inputs.currentFlags = static_cast<std::uint32_t>(app->ownershipFlags);

        // Remember every managed card, not only the ones this fill had to touch:
        // the owned bit is often the client's own, set because package 0 answers
        // the app as licensed, and it must still be taken back on removal.
        if (inputs.managed)
            TrackManaged(app->appId);

        const auto decision = policy::DecideOwnershipWrite(inputs);
        if (decision.action == policy::OwnershipAction::Stamp) {
            app->ownershipFlags = static_cast<steam::AppOwnershipFlags>(
                policy::ApplyOwned(static_cast<std::uint32_t>(app->ownershipFlags)));
            g_state.overview.writes.fetch_add(1, std::memory_order_relaxed);
        } else if (decision.action == policy::OwnershipAction::Clear) {
            app->ownershipFlags = static_cast<steam::AppOwnershipFlags>(
                policy::ApplyCleared(static_cast<std::uint32_t>(app->ownershipFlags)));
            ForgetTracking(app->appId);
            g_state.overview.clears.fetch_add(1, std::memory_order_relaxed);
            AC_LOG_INFO(kModule, "Overview: cleared the owned bit of app %u after its .lua was removed.", app->appId);
            status::Write();
        }
    }
    return o_FillInAppOverview(self, appOverview, app);
}

// ---- Recheck of a changed app ---------------------------------------------
// The client's own "this app's AppInfo or depot config is stale" entry point.
// Aether never invents an id or a flag: every call carries an id a Lua change
// selected and the client's AppInfoOrConfig bit.

// This detour exists only to learn the object that owns the app list. The latest
// `self` seen is stored and the arguments are forwarded unchanged: rewriting
// them would change what the client records, and a hook that edits is no longer
// a seam that can be trusted with an unknown call shape. Keeping the latest
// rather than the first matters because a rebuilt client UI or an account switch
// replaces the owner, and the old pointer would dangle.
using MarkAppChange_t = void* (*)(void*, steam::AppId, std::uint32_t);
MarkAppChange_t o_MarkAppChange = nullptr;

void* h_MarkAppChange(void* self, steam::AppId app, std::uint32_t changeFlags) {
    g_state.overview.appChangeSource.store(self, std::memory_order_release);
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
        // The ids stay queued: an owner is learned from the latest MarkAppChange
        // the client makes on its own, and a later frame drains them then.
        if (pending) {
            AC_LOG_WARN_ONCE(kModule,
                             "App recheck is waiting: MarkAppChange has not run, so no app-list owner is known.");
            // The reload's own write ran before this frame, so the status file
            // would keep claiming the queue is empty without this one.
            if (!g_noSourceStatusWritten.exchange(true, std::memory_order_relaxed))
                status::Write();
        }
        return;
    }

    bool handed = false;
    for (std::size_t i = 0; i < kMaxRechecksPerFrame; ++i) {
        steam::AppId app = 0;
        {
            // Only the pop is guarded. The client call below may take its own
            // locks and must never wait for the worker that is pushing.
            std::lock_guard lock(g_state.overview.recheckMutex);
            if (!g_state.overview.rechecks.Pop(app))
                break;
        }
        // A card Aether took back has no object left in the client: the id needs
        // the "added" change, not a config change, or the re-added `.lua` never
        // gets a card again in this session.
        bool hidden = false;
        {
            std::lock_guard lock(g_state.overview.stampMutex);
            hidden = g_state.overview.hidden.count(app) != 0;
        }
        o_MarkAppChange(source, app, policy::RecheckFlags(hidden));
        g_state.overview.recheckCalls.fetch_add(1, std::memory_order_relaxed);
        handed = true;
        if (hidden) {
            AC_LOG_INFO(kModule, "Asked the client to add app %u back to the library.", app);
        } else {
            AC_LOG_INFO(kModule, "Asked the client to re-evaluate app %u after its manifest pins changed.", app);
        }
    }
    // `overview_rechecks`, `overview_recheck_pending` and `recheck_state` are the
    // evidence that the card was rebuilt, and every write before this frame
    // predates the calls above, so the drain publishes its own result.
    if (handed)
        status::Write();
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
            // The capacity check runs first, so an id the queue already holds is
            // counted here too: the WARN says "not accepted", not "lost".
            if (g_state.overview.rechecks.Size() >= policy::RecheckQueue::kCapacity) {
                ++refused;
                continue;
            }
            g_state.overview.rechecks.Push(app);
        }
    }
    if (refused != 0) {
        AC_LOG_WARN_ONCE(kModule,
                         "App recheck queue full: %zu request(s) not accepted; a refused id stays unrechecked "
                         "until that app's Lua changes again.",
                         refused);
    }
}

bool AppRecheckSupported() { return o_MarkAppChange != nullptr && o_CSteamUIAppControllerRunFrame != nullptr; }

void RequestAppRetirement(const std::vector<steam::AppId>& ids) {
    if (ids.empty())
        return;
    std::size_t refused = 0;
    std::size_t accepted = 0;
    {
        std::lock_guard lock(g_state.overview.stampMutex);
        for (const steam::AppId app : ids) {
            // Capacity first, then the duplicate test: an id already retired is
            // still going to be cleared, so it is not a refusal.
            if (g_state.overview.retired.size() >= policy::kMaxTrackedApps &&
                g_state.overview.retired.count(app) == 0) {
                ++refused;
                continue;
            }
            if (g_state.overview.retired.insert(app).second)
                ++accepted;
        }
    }
    if (refused != 0) {
        AC_LOG_WARN_ONCE(kModule,
                         "App retirement set full: %zu id(s) not recorded; those cards stay until the next start.",
                         refused);
    }
    if (accepted != 0) {
        AC_LOG_INFO(kModule, "Retired %zu app(s) after a .lua removal.", accepted);
        // `overview_retired` is the evidence the next card fill acted on; the
        // reload's own status write ran before this call.
        status::Write();
    }
}

} // namespace ac::hooks::steamui
