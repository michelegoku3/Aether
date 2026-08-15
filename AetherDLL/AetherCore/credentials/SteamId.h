#pragma once

#include <cstdint>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// SteamID resolution for ownership spoofing.
//
// When Steam (or a Steam-DRM wrapper) asks "who owns this app?", we must answer
// with the SteamID of the account currently using this machine. Cached per-app
// values are only a fallback when the live identity cannot be resolved (Steam
// closed, no ActiveUser, empty userdata). Preferring cache first is wrong:
// switching Steam accounts would keep spoofing the previous owner's id.
//
// LumaCore scattered this logic across Ticket.cpp and CmdUser.cpp; AetherCore
// keeps it in one cohesive module. It is consumed by the ownership/IPC layer,
// NOT by any achievement code (which is excluded from this project).
// ---------------------------------------------------------------------------
namespace ac::steamid {

// SteamID64 of the currently logged-in user, or 0 if none can be resolved.
// Tries HKCU\...\ActiveProcess\ActiveUser (live while Steam runs), then falls
// back to the most recently modified userdata\<accountId>\ folder.
std::uint64_t GetActiveSteamId64();

// Best SteamID64 to present as the owner of appId, or 0 if unknown.
// Resolution order:
//   1. Active identity (GetActiveSteamId64) — source of truth; also refreshes
//      Apps\<appId>\SteamID so local cache tracks the current account.
//   2. HKCU\...\Apps\<appId>\SteamID  (stale-safe fallback when Steam is down)
//   3. the SteamID embedded in the cached AppOwnershipTicket
//   4. a userdata\<accountId>\<appId>\ folder (the user has played it)
// Only configured apps (present in depotKeys) are spoofed; others return 0.
std::uint64_t GetSpoofSteamId(steam::AppId appId);

}  // namespace ac::steamid
