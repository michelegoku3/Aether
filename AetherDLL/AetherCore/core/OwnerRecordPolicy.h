#pragma once

#include <cstdint>

// ---------------------------------------------------------------------------
// Pure decision table for the library-owner policy.
//
// Steam derives `strOwnerSteamID` for an app from `AppState.LastOwner`, the
// account that last wrote app state (install, update, validate, account
// switch). Inside a Steam Family the app-details page renders the generic
// "From your Steam Family library" banner whenever the app looks subscribed,
// the recorded owner is not the signed-in account and the family holds no
// different copies. That banner is correct for a real family license, but on a
// Lua-provided app nobody in the family owns it is a bogus attribution.
//
// The policy therefore keeps an owner record per app:
//
//   * spoofed app (Aether provides it now) -> write the ACTIVE account, so
//     Steam attributes the app to this login and cannot render the family
//     banner;
//   * family-shared app (Steam reports a real family license) -> restore the
//     value Steam had before Aether touched the manifest, so Steam's own banner
//     can name a local owner again; never invent an owner;
//   * app Aether no longer spoofs (owned, or its .lua was removed) -> keep the
//     stamp when it is the active account (a truthful last owner), otherwise
//     restore the original;
//   * never managed and never written -> never touched.
//
// The record stores what Aether wrote (0 = the entry was removed) and the
// pre-Aether value, so a restore can put Steam's data back byte-for-byte. The
// active account is read from Steam (credentials/SteamId), never from Desk.
//
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
    std::uint64_t activeSteamId = 0; // signed-in account; 0 when unknown
    bool recorded = false;
    OwnerRecord record{};
    std::uint64_t currentLastOwner = 0; // 0 when absent or unparsable
};

struct OwnerPlan {
    OwnerAction action = OwnerAction::Skip;
    std::uint64_t owner = 0;
    bool dropRecord = false; // the (appId -> record) entry is obsolete
};

inline OwnerPlan PlanOwnerAction(const OwnerContext& c) {
    OwnerPlan plan;

    // Opt-in cosmetic banner: a listed spoofed app names the configured owner.
    if (c.bannerEnabled && c.managed && c.listed && c.bannerSteamId != 0) {
        plan.action = OwnerAction::Set;
        plan.owner = c.bannerSteamId;
        return plan;
    }

    // Steam reports a real family license: put the manifest back the way Steam
    // had it. The record proves the manifest value is Aether's; a different
    // value was written by Steam (family change, reinstall) and is left alone.
    if (c.familyShared) {
        if (!c.recorded)
            return plan;
        plan.dropRecord = true;
        if (c.currentLastOwner != c.record.written) {
            plan.action = OwnerAction::Skip;
            return plan;
        }
        if (c.record.originalKnown && c.record.original != 0) {
            plan.action = OwnerAction::Set;
            plan.owner = c.record.original;
        } else {
            plan.action = OwnerAction::Remove;
        }
        return plan;
    }

    // Spoofed app: Steam must attribute it to this login. Before login the
    // active account is unknown, so the leftover is dropped instead.
    if (c.managed) {
        if (c.activeSteamId != 0) {
            plan.action = OwnerAction::Set;
            plan.owner = c.activeSteamId;
        } else {
            plan.action = OwnerAction::Remove;
        }
        return plan;
    }

    // Aether wrote this app before and no longer spoofs it.
    if (!c.recorded)
        return plan;
    if (c.currentLastOwner != c.record.written) {
        plan.dropRecord = true;
        return plan;
    }
    if (c.activeSteamId != 0 && c.record.written == c.activeSteamId)
        return plan; // our stamp is the real last owner: keep it and the record
    plan.dropRecord = true;
    if (c.record.originalKnown && c.record.original != 0) {
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
inline OwnerRecord RecordAfterWrite(const OwnerRecord& previous, bool hasPrevious, std::uint64_t currentLastOwner,
                                    std::uint64_t written) {
    OwnerRecord next;
    if (!hasPrevious || currentLastOwner != previous.written) {
        next.originalKnown = currentLastOwner != 0;
        next.original = currentLastOwner;
    } else {
        next.originalKnown = previous.originalKnown;
        next.original = previous.original;
    }
    next.written = written;
    return next;
}

} // namespace ac::appmanifest
