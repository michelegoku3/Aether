#pragma once

#include <cstddef>

// ---------------------------------------------------------------------------
// AppManifestOwner — apply Steam's per-app "last owner" policy.
//
// Steam renders "From the library of <owner>" (sp.js
// `#AppDetails_InSharedLibrary`) for any installed app the current account does
// not subscribe to whose recorded owner is a different account. For Lua-provided
// apps that banner is always bogus: Aether spoofs ownership, yet the appmanifest
// keeps the `LastOwner` written by whichever account last touched the install.
//
// Two policies, both applied to installed apps Aether actually spoofs
// (`luadata::HasDepot`):
//   * default: remove `AppState.LastOwner`. Steam then has nobody to attribute
//     the app to and the banner cannot render for any login. This is
//     account-independent and survives account switches.
//   * `[library] owner_banner = true`: for the appids listed in
//     `owner_banner_apps`, write `LastOwner = owner_banner_steamid` so Steam
//     renders the real banner with that account's persona. All other apps keep
//     the entry removed.
//
// Values Aether writes are recorded in `<steam>\aethercore\owner_state.txt`.
// The record is what lets a later pass remove its own stamp after the app
// leaves the managed set (its .lua was removed) without ever touching a
// LastOwner Steam wrote itself: a value that no longer matches the record is
// Steam's and is left alone.
//
// Runs at boot (before Steam loads app state), after login, and after Steam
// rewrites a manifest (install/update/validate).
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
    std::size_t errors = 0;
};

// Applies the configured policy to every installed managed app.
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
