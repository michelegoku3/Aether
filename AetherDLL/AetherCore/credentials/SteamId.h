#pragma once

#include <cstdint>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// SteamID resolution for ownership spoofing.
//
// When Steam (or a Steam-DRM wrapper) asks "who owns this app?", we must answer
// with the SteamID Steam itself associates with that app, otherwise the DRM
// layer rejects the mismatch. This module derives that id from local sources,
// in decreasing order of reliability.
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
//   1. HKCU\...\Apps\<appId>\SteamID  (REG_SZ written by Steam)
//   2. the SteamID embedded in the cached AppOwnershipTicket
//   3. a userdata\<accountId>\<appId>\ folder (the user has played it)
// Only configured apps (present in depotKeys) are spoofed; others return 0.
std::uint64_t GetSpoofSteamId(steam::AppId appId);

}  // namespace ac::steamid
