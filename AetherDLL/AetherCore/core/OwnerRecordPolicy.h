#pragma once

#include <cstdint>

// ---------------------------------------------------------------------------
// Pure decision table for the cosmetic library-owner policy.
//
// Steam stamps `AppState.LastOwner` with whichever account last wrote app state
// (install, update, validate, account switch). On an app Aether spoofs that
// value can only produce a bogus "From the library of <account>" banner, so the
// default policy drops it whether Steam or Aether put it there.
//
// The opt-in banner still records (appId -> value we wrote): that record lets a
// later pass remove our stamp after the app leaves the managed set (.lua
// removed), where the value can still be told apart from a real Steam record,
// which is never touched:
//
//   * managed now -> remove the value (banner opt-in still wins);
//   * no longer managed and still our recorded value -> remove it;
//   * no longer managed and Steam rewrote it (real license, family change,
//     reinstall) -> drop the stale record and leave Steam's data alone;
//   * never managed and never written by Aether -> never touched.
//
// Kept header-only and free of globals so the matrix is unit-testable without
// Steam, Lua state or a filesystem.
// ---------------------------------------------------------------------------
namespace ac::appmanifest {

enum class OwnerAction {
    Skip,   // leave the manifest and any record alone
    Set,    // write `owner` as LastOwner
    Remove, // drop the LastOwner Aether wrote
};

struct OwnerPlan {
    OwnerAction action = OwnerAction::Skip;
    std::uint64_t owner = 0;
    bool dropRecord = false; // the (appId -> owner) record is obsolete
};

// `recordedOwner` is 0 when Aether never wrote this app; `currentLastOwner` is
// the value currently in the manifest (0 when absent or unparsable).
inline OwnerPlan PlanOwnerAction(bool bannerEnabled, bool managed, bool listed, std::uint64_t bannerSteamId,
                                 std::uint64_t recordedOwner, std::uint64_t currentLastOwner) {
    OwnerPlan plan;
    if (bannerEnabled && managed && listed && bannerSteamId != 0) {
        plan.action = OwnerAction::Set;
        plan.owner = bannerSteamId;
        return plan;
    }

    // A managed app is spoofed right now: Steam cannot legitimately attribute
    // it to another account, so any LastOwner left behind is the bogus one that
    // renders the banner. Drop it even when Aether never wrote it.
    if (managed) {
        plan.action = OwnerAction::Remove;
        return plan;
    }

    if (recordedOwner == 0)
        return plan; // not ours: a real Steam record stays untouched

    if (currentLastOwner == recordedOwner) {
        plan.action = OwnerAction::Remove;
        return plan;
    }

    // Already gone, or Steam rewrote it: stop tracking, change nothing.
    plan.dropRecord = true;
    return plan;
}

} // namespace ac::appmanifest
