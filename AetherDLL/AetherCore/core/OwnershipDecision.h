#pragma once

#include <cstdint>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Pure ownership decision table for CheckAppOwnership.
//
// Steam's own answer is the source of truth only until Aether has classified
// the app. Package 0 is filled after that first answer, so later calls can see
// a synthetic "owned" result that Aether itself created. The sticky state in
// `spoofed` and `familyLenderAccountId` tells the table to ignore that
// synthetic result and keep the classification:
//   * a released app present in more than one package is Steam-provided;
//   * among those, borrowed/familyShared marks a real Steam Family license;
//   * everything else is Aether's to spoof from the .lua data.
//
// The app-details page reads the owner from `AppOwnership.steamId32`. A spoofed
// app must carry the signed-in account (or the opt-in banner account) there or
// Steam renders the generic "From your Steam Family library" banner. A family
// app stays playable through package 0 but is attributed to the real lender,
// so its banner survives the synthetic license.
// ---------------------------------------------------------------------------
namespace ac::ownership {

struct OwnershipFacts {
    bool steamResult = false;
    std::uint32_t existInPackageNums = 0;
    steam::AppReleaseState releaseState = steam::AppReleaseState::Invalid;
    bool borrowed = false;
    bool familyShared = false;
    // The app is configured by a .lua (Aether has data for it).
    bool hasDepot = false;
    // Account Aether puts in AppOwnership.steamId32 when it spoofs; 0 when the
    // live account is not known yet.
    std::uint32_t spoofOwnerAccountId = 0;
    // Sticky Aether classification: Steam did not provide the app and Aether
    // already answered as owned. Package 0 now makes Steam report owned too.
    bool spoofed = false;
    // Sticky real family lender, non-zero once a family license was observed.
    // The app is kept in package 0 for simultaneous play, but attributed to
    // this account so Steam keeps the family banner.
    std::uint32_t familyLenderAccountId = 0;
};

enum class OwnershipAction {
    PassThrough, // leave Steam's answer untouched
    Spoof,       // answer as owned by spoofOwnerAccountId
};

struct OwnershipDecision {
    OwnershipAction action = OwnershipAction::PassThrough;
    bool steamOwned = false;          // pass-through: mark genuinely owned
    bool steamFamilyShared = false;   // pass-through: mark real family license
    bool familyBanner = false;        // spoof: keep the family bit for the banner
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

    // Sticky classification wins over Steam's current answer. Once package 0
    // carries the app, Steam reports it as owned; re-classifying it as a real
    // Steam license would lose the spoof or the family banner.
    if (facts.spoofed || facts.familyLenderAccountId != 0) {
        OwnershipDecision decision;
        decision.action = OwnershipAction::Spoof;
        decision.ownerAccountId = facts.spoofOwnerAccountId;
        // A real family app stays a family app in the UI even though package 0
        // makes the license answer owned; that bit is what renders the banner.
        decision.familyBanner = facts.familyLenderAccountId != 0;
        return decision;
    }

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
