#pragma once

#include <cstddef>

// ---------------------------------------------------------------------------
// AppManifestOwner — apply Steam's per-app "last owner" policy.
//
// Steam renders "From the library of <owner>" (sp.js
// `#AppDetails_InSharedLibrary`) and, inside a Steam Family, the generic "From
// your Steam Family library" banner (`#FamilyGroup_GameAvailable`) from the
// app's recorded owner: `AppState.LastOwner`, the account that last wrote app
// state. On a Lua-provided app nobody in the family owns, that attribution is
// bogus; on a genuinely family-shared app it is Steam's own banner.
//
// Three rules, applied to installed apps Aether spoofs (`luadata::HasDepot`),
// has written before, or Steam reports as family-shared:
//   * spoofed apps get `AppState.LastOwner = <active account>`, so Steam
//     attributes the app to this login and cannot render the family banner.
//     This is account-specific and re-applied after login and account
//     switches.
//   * `[library] owner_banner = true`: for the appids listed in
//     `owner_banner_apps`, `LastOwner = owner_banner_steamid` shows the banner
//     with that account's persona.
//   * family-shared apps are restored to the `LastOwner` Steam had before
//     Aether touched the manifest, so Steam's own banner can name a local
//     owner. A value Steam rewrote in the meantime is Steam's and is left
//     alone.
//
// Values Aether writes are recorded in `<steam>\aethercore\owner_state.txt`
// together with the pre-Aether value: that record lets a later pass restore
// Steam's data after the app leaves the managed set (its .lua was removed) or
// turns out to be family-shared.
//
// Runs at boot (before Steam loads app state), after login, and after Steam
// classifies an app as family-shared (install/update/validate rewrites).
//
// Best-effort and idempotent: unwritable/locked manifests are skipped and
// reported, never retried in a loop.
// ---------------------------------------------------------------------------
namespace ac::appmanifest {

struct OwnerPolicyStats {
    std::size_t scanned = 0;
    std::size_t applied = 0; // LastOwner written (active account or banner opt-in)
    std::size_t removed = 0; // LastOwner dropped
    std::size_t unchanged = 0;
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
