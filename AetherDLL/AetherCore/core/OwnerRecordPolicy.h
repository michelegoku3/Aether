#pragma once

#include <cstdint>

// ---------------------------------------------------------------------------
// Pure decision table for the appmanifest owner record.
//
// Steam reports `strOwnerSteamID` from the ownership record's owner account id
// (`AppOwnership.steamId32`): the family lender on a shared license, or the
// signed-in account Aether writes for a spoofed app. The app-details page
// renders the generic "From your Steam Family library" banner only while that
// owner is not the signed-in account.
//
// `AppState.LastOwner` is Steam's data, not Aether's attribution mechanism.
// Steam reads it at sign-in; an active-account stamp makes a genuine family
// license look like a local install, so Steam stops reporting the lender and
// its banner disappears. The policy therefore:
//
//   * spoofed app -> remove the stale/Aether stamp; the ownership hook carries
//     the attribution through AppOwnership.steamId32;
//   * family-shared app (Steam reports a real family license) -> restore the
//     value Steam had before Aether touched the manifest;
//   * app Aether no longer spoofs -> restore the original, never keep a stamp;
//   * opt-in `[library] owner_banner` -> write the configured account (the only
//     case Aether writes LastOwner).
//
// A restore never writes the signed-in account either: an old record may carry
// it as the pre-Aether value, and restoring it would hide the banner again.
//
// The record stores what Aether wrote (0 = the entry was removed) and the
// pre-Aether value, so a restore can put Steam's data back byte-for-byte.
// Kept header-only and free of globals so the matrix is unit-testable without
// Steam, Lua state or a filesystem.
// ---------------------------------------------------------------------------
namespace ac::appmanifest {

enum class OwnerAction {
    Skip,   // leave the manifest and any record alone
    Set,    // write `owner` as LastOwner
    Remove, // drop the LastOwner entry
};

struct OwnerRecord {
    // Value Aether last wrote; 0 means Aether removed the entry.
    std::uint64_t written = 0;
    // The value Steam had before the first Aether write (false = no entry).
    bool originalKnown = false;
    std::uint64_t original = 0;
};

struct OwnerContext {
    bool bannerEnabled = false; // [library] owner_banner
    bool managed = false;       // Aether spoofs this app now (luadata::HasDepot)
    bool familyShared = false;  // Steam reports a real family license
    bool listed = false;        // appid is in owner_banner_apps
    std::uint64_t bannerSteamId = 0;
    bool recorded = false;
    OwnerRecord record{};
    std::uint64_t currentLastOwner = 0; // 0 when absent or unparsable
    // Signed-in account, 0 before login. A restore never writes it: an
    // active-account stamp is what hid the family banner in the first place.
    std::uint64_t activeSteamId = 0;
    // AppState.StateFlags says Steam is updating the app right now.
    bool updateRunning = false;
    // This session already settled this app with the same plan.
    bool alreadyApplied = false;
};

// A pre-Aether value is restorable only when it is a real owner and not the
// account Steam reads at sign-in.
inline bool RestorableOwner(const OwnerContext& c) {
    return c.record.originalKnown && c.record.original != 0 && c.record.original != c.activeSteamId;
}

struct OwnerPlan {
    OwnerAction action = OwnerAction::Skip;
    std::uint64_t owner = 0;
    bool dropRecord = false; // the (appId -> record) entry is obsolete
};

inline OwnerPlan PlanOwnerAction(const OwnerContext& c) {
    OwnerPlan plan;

    // Steam rewrites the manifest when an update starts and when it ends. A
    // write of ours in between invalidates the state Steam is working from,
    // and Steam overwrites it moments later: defer to a later pass.
    if (c.updateRunning)
        return plan;

    // The boot pass and the post-login pass used to write each other's result
    // back and forth. Once this session settled the app, leave it alone until
    // the plan changes (settings, .lua reload) or Steam rewrites the manifest.
    if (c.alreadyApplied)
        return plan;

    // Opt-in cosmetic banner: the only case Aether writes an owner.
    if (c.bannerEnabled && c.managed && c.listed && c.bannerSteamId != 0) {
        plan.action = OwnerAction::Set;
        plan.owner = c.bannerSteamId;
        return plan;
    }

    // Real family license: put the manifest back the way Steam had it. The
    // record proves the manifest value is Aether's; a different value was
    // written by Steam (family change, reinstall) and is left alone.
    if (c.familyShared) {
        if (!c.recorded)
            return plan;
        plan.dropRecord = true;
        if (c.currentLastOwner != c.record.written) {
            plan.action = OwnerAction::Skip;
            return plan;
        }
        if (RestorableOwner(c)) {
            plan.action = OwnerAction::Set;
            plan.owner = c.record.original;
        } else {
            plan.action = OwnerAction::Remove;
        }
        return plan;
    }

    // Spoofed app: never claim LastOwner. Removing the stamp lets Steam report
    // a real family license again at the next sign-in; the hook attributes the
    // app through AppOwnership.steamId32.
    if (c.managed) {
        if (c.currentLastOwner != 0)
            plan.action = OwnerAction::Remove;
        return plan;
    }

    // Aether wrote this app before and no longer spoofs it: restore Steam's
    // value. Never keep an active-account stamp.
    if (!c.recorded)
        return plan;
    if (c.currentLastOwner != c.record.written) {
        plan.dropRecord = true;
        return plan;
    }
    plan.dropRecord = true;
    if (RestorableOwner(c)) {
        plan.action = OwnerAction::Set;
        plan.owner = c.record.original;
    } else {
        plan.action = OwnerAction::Remove;
    }
    return plan;
}

// The record to persist after `written` replaces `currentLastOwner`.
//
// The pre-Aether value is captured on the first write and whenever Steam
// rewrote the manifest since Aether's last write (a fresh Steam value beats a
// stale one); a later pass over Aether's own stamp keeps the captured original.
// The signed-in account is never a restorable original: it is the value that
// made a genuine family license look locally owned.
inline OwnerRecord RecordAfterWrite(const OwnerRecord& previous, bool hasPrevious, std::uint64_t currentLastOwner,
                                    std::uint64_t activeSteamId, std::uint64_t written) {
    OwnerRecord next;
    if (!hasPrevious || currentLastOwner != previous.written) {
        next.originalKnown = currentLastOwner != 0 && currentLastOwner != activeSteamId;
        next.original = next.originalKnown ? currentLastOwner : 0;
    } else {
        next.originalKnown = previous.originalKnown && previous.original != activeSteamId;
        next.original = next.originalKnown ? previous.original : 0;
    }
    next.written = written;
    return next;
}

} // namespace ac::appmanifest
