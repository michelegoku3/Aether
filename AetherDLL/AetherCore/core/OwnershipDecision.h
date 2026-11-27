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
// Steam renders the generic "From your Steam Family library" banner. A banner
// app is attributed to the stamp account so its banner survives the synthetic
// license; a real Steam Family licence is never spoofed at all.
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
    // Sticky family lender, non-zero once a family answer was observed. It names
    // the banner account for an app Aether provides, and the real lender for a
    // Valve licence (see valveFamilyLicense).
    std::uint32_t familyLenderAccountId = 0;
    // The family answer is a licence Valve granted to another account. Nothing
    // in package 0 or in the answer may contradict it.
    bool valveFamilyLicense = false;
    // True when Steam's answer came from package 0 (packageId == 0 and
    // freeLicense): package 0 is where Aether seeds unowned apps and can count
    // >1 during license cache rebuilds without representing a real Steam buy.
    bool freePackage0Only = false;
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
    return facts.hasDepot && facts.steamResult && !facts.freePackage0Only && facts.existInPackageNums > 1 &&
           facts.releaseState == steam::AppReleaseState::Released;
}

inline OwnershipDecision DecideOwnership(const OwnershipFacts& facts) {
    // No Aether data: nothing to spoof and nothing to mark.
    if (!facts.hasDepot)
        return {};

    // A real Steam Family licence belongs to Valve. Answering owned here is what
    // makes the library offer PLAY for a session the CM stops minutes later, so
    // the decision is Steam's own answer, marked as the family share it is.
    // Even if Aether speculatively spoofed the app at boot before Steam loaded
    // CM family state, a fresh unmasked borrowed/familyShared answer wins.
    const bool steamReportedFamily = facts.hasDepot && facts.steamResult && facts.existInPackageNums > 1 &&
                                     facts.releaseState == steam::AppReleaseState::Released &&
                                     (facts.borrowed || facts.familyShared);
    if (facts.valveFamilyLicense || (facts.familyLenderAccountId == 0 && steamReportedFamily)) {
        OwnershipDecision decision;
        decision.action = OwnershipAction::PassThrough;
        decision.steamFamilyShared = true;
        return decision;
    }

    // Sticky classification wins over Steam's current answer. Once package 0
    // carries the app, Steam reports it as owned; re-classifying it as a real
    // Steam license would lose the spoof or the family banner.
    if (facts.spoofed || facts.familyLenderAccountId != 0) {
        OwnershipDecision decision;
        decision.action = OwnershipAction::Spoof;
        decision.ownerAccountId = facts.spoofOwnerAccountId;
        // A banner app stays a family app in the UI even though package 0
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

// Resolves the family lender when PassThrough handles a real family share.
// Only accepts reportedSteamId32 when Steam itself flagged the response as
// borrowed/familyShared and it is not the active account (Package 0 entries
// carry activeAccountId with borrowed=0 and familyShared=0).
inline std::uint32_t ResolveObservedFamilyLender(bool steamReportedFamily, std::uint32_t reportedSteamId32,
                                                 std::uint32_t existingLender, std::uint32_t activeAccountId) {
    if (steamReportedFamily && reportedSteamId32 != 0 && reportedSteamId32 != activeAccountId)
        return reportedSteamId32;
    if (existingLender != 0 && existingLender != activeAccountId)
        return existingLender;
    return 0;
}

// True when Steam already proves a direct non-shared license on the active
// user's account via a non-zero real package id (paid or free, e.g. free base
// app + DLC lua). FamilyLenders must not reclassify these as RealShare.
inline bool IsDirectlyLicensedByActiveAccount(bool steamResult, std::uint32_t existInPackageNums,
                                              steam::AppReleaseState releaseState, std::uint32_t packageId,
                                              bool borrowed, bool familyShared, bool freeLicense) {
    return steamResult && releaseState == steam::AppReleaseState::Released && packageId != 0 && !borrowed &&
           !familyShared && (existInPackageNums > 1 || freeLicense);
}

} // namespace ac::ownership
