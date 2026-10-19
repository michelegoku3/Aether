#pragma once

#include <cstddef>

// ---------------------------------------------------------------------------
// AppManifestOwner — drop Steam's per-app "last owner" record.
//
// Steam renders "From the library of <owner>" (sp.js
// `#AppDetails_InSharedLibrary`) for any installed app the current account does
// not subscribe to whose recorded owner is a different account. For Lua-provided
// apps that banner is always bogus: Aether spoofs ownership, yet the appmanifest
// keeps the `LastOwner` written by whichever account last touched the install.
//
// Steam reads app state at startup, before it knows which account will log in,
// so writing "the current account" is only a guess about the previous session
// and simply flips the banner to the other account at every switch. Removing the
// entry has no such dependency: with no owner recorded, Steam has nobody to
// attribute the app to and the banner cannot render for any login.
//
// This module removes `AppState.LastOwner` for every installed app Aether
// actually spoofs. Runs at boot (before Steam loads app state), after login, and
// after Steam rewrites a manifest (install/update/validate).
//
// Best-effort and idempotent: unwritable/locked manifests are skipped and
// reported, never retried in a loop.
// ---------------------------------------------------------------------------
namespace ac::appmanifest {

struct StripStats {
    std::size_t scanned = 0;
    std::size_t removed = 0;
    std::size_t unchanged = 0;
    std::size_t errors = 0;
};

// Removes `AppState.LastOwner` from every installed app that needs spoofing
// (`luadata::HasDepot`). Returns what it did. Never throws.
StripStats StripInstalledAppOwners();

// Runs one pass on the worker queue. Coalesces repeated calls
// (MarkLicenseAsChanged fires several times per login) into at most one pending
// job.
void RequestStrip();

// One boot-time pass. Must happen before Steam loads app state for the session;
// the post-login pass covers the rest.
void StripAtStartup();

} // namespace ac::appmanifest
