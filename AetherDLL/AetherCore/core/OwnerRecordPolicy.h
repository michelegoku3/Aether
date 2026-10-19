#pragma once

#include <cstdint>

// ---------------------------------------------------------------------------
// Pure decision table for the cosmetic library-owner policy.
//
// The policy writes `AppState.LastOwner` on installed apps Aether spoofs, and
// must be able to undo exactly those writes later. It therefore keeps a record
// of (appId -> value we wrote) and only ever removes a value that is still the
// one we recorded:
//
//   * Steam rewrote the value (real license, family change, reinstall) -> drop
//     the stale record and leave Steam's data alone;
//   * the app left the managed set (its .lua was removed) -> remove the value
//     WE wrote, so the banner does not survive the feature;
//   * an app Aether never wrote -> never touched.
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
