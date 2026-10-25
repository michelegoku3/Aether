#pragma once

#include <cstdint>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Pure ownership decision table for CheckAppOwnership.
//
// Steam's own answer is the source of truth:
//   * a released app present in more than one package is Steam-provided;
//   * among those, borrowed/familyShared marks a real Steam Family license;
//   * everything else is Aether's to spoof from the .lua data.
//
// The app-details page reads the owner from `AppOwnership.steamId32`. A spoofed
// app must carry the signed-in account (or the opt-in banner account) there or
// Steam renders the generic "From your Steam Family library" banner.
// Steam-provided apps pass through untouched: a real family license keeps its
// lender and its banner, and its package/borrow state stays Steam's own.
// ---------------------------------------------------------------------------
namespace ac::ownership {

struct OwnershipFacts {
    bool steamResult = false;
    std::uint32_t existInPackageNums = 0;
    steam::AppReleaseState releaseState = steam::AppReleaseState::Invalid;
    bool borrowed = false;
    bool familyShared = false;
    bool hasDepot = false;
    // Account Aether puts in AppOwnership.steamId32 when it spoofs; 0 when the
    // live account is not known yet.
    std::uint32_t spoofOwnerAccountId = 0;
};

enum class OwnershipAction {
    PassThrough, // leave Steam's answer untouched
    Spoof,       // answer as owned by spoofOwnerAccountId
};

struct OwnershipDecision {
    OwnershipAction action = OwnershipAction::PassThrough;
    bool steamOwned = false;          // pass-through: mark genuinely owned
    bool steamFamilyShared = false;   // pass-through: mark real family license
    std::uint32_t ownerAccountId = 0; // spoof: value for steamId32
};

// Steam already provides the game: a released app present in more than one
// package. The hook uses this before reading the live account, which costs a
// registry lookup.
inline bool IsSteamProvided(const OwnershipFacts& facts) {
    return facts.hasDepot && facts.steamResult && facts.existInPackageNums > 1 &&
           facts.releaseState == steam::AppReleaseState::Released;
}

inline OwnershipDecision DecideOwnership(const OwnershipFacts& facts) {
    // No Aether data: nothing to spoof and nothing to mark.
    if (!facts.hasDepot)
        return {};

    if (IsSteamProvided(facts)) {
        OwnershipDecision decision;
        decision.action = OwnershipAction::PassThrough;
        decision.steamFamilyShared = facts.borrowed || facts.familyShared;
        decision.steamOwned = !decision.steamFamilyShared;
        return decision;
    }

    OwnershipDecision decision;
    decision.action = OwnershipAction::Spoof;
    decision.ownerAccountId = facts.spoofOwnerAccountId;
    return decision;
}

} // namespace ac::ownership
