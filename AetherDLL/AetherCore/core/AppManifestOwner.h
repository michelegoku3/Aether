#pragma once

#include <cstddef>

// ---------------------------------------------------------------------------
// AppManifestOwner — keep `AppState.LastOwner` out of Aether's attribution.
//
// Steam reads `AppState.LastOwner` at sign-in and renders "From the library of
// <owner>" (sp.js `#AppDetails_InSharedLibrary`) or, inside a Steam Family, the
// generic "From your Steam Family library" banner
// (`#FamilyGroup_GameAvailable`) from the account it names. Aether never writes
// the signed-in account there: an active-account stamp makes a genuine family
// license look like a local install, so Steam stops reporting the lender and
// its banner disappears.
//
// The app-details page gets the owner from `AppOwnership.steamId32` instead.
// `OwnershipHooks` writes the signed-in account (or the opt-in banner account)
// for a spoofed app, and leaves a Steam-provided answer untouched.
//
// Policy, applied to installed apps Aether spoofs (`luadata::HasDepot`), has
// written before, or Steam reports as family-shared:
//   * spoofed app: remove any `LastOwner` entry. The hook carries the
//     attribution through `AppOwnership.steamId32`.
//   * family-shared app: restore the `LastOwner` Steam had before Aether
//     touched the manifest, so Steam's own banner can name a local owner. A
//     value Steam rewrote in the meantime is Steam's and is left alone.
//   * app Aether no longer spoofs: restore the original value, never keep an
//     active-account stamp.
//   * a restore never writes the signed-in account: an old record may carry it
//     as the pre-Aether value, and restoring it would hide the banner again.
//   * `[library] owner_banner = true`: for the appids listed in
//     `owner_banner_apps`, `LastOwner = owner_banner_steamid` shows the banner
//     with that account's persona. This is the only case Aether writes it.
//
// Values Aether writes (including a removal, recorded as 0) are stored in
// `<steam>\aethercore\owner_state.txt` with the pre-Aether value: that record
// lets a later pass restore Steam's data after the app leaves the managed set
// (its .lua was removed) or turns out to be family-shared.
//
// Runs at boot (before Steam loads app state; the policy only removes or
// restores, so it needs no account yet), after login, and after Steam
// classifies an app as family-shared (install/update/validate rewrites).
//
// Best-effort and idempotent: unwritable/locked manifests are skipped and
// reported, never retried in a loop.
// ---------------------------------------------------------------------------
namespace ac::appmanifest {

struct OwnerPolicyStats {
    std::size_t scanned = 0;
    std::size_t applied = 0; // LastOwner written (banner opt-in)
    std::size_t removed = 0; // LastOwner dropped
    std::size_t unchanged = 0;
    std::size_t deferred = 0; // update in flight or already settled this session
    std::size_t errors = 0;
};

// Applies the configured policy to every installed app the policy owns.
// Never throws.
OwnerPolicyStats ApplyInstalledAppOwners();

// Runs one pass on the worker queue. Coalesces repeated calls
// (MarkLicenseAsChanged fires several times per login) into at most one pending
// job.
void RequestOwnerApply();

// One boot-time pass. Must happen before Steam loads app state for the session;
// the post-login pass covers the rest.
void ApplyOwnersAtStartup();

} // namespace ac::appmanifest
