#pragma once

#include <cstdint>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Real Steam Family lender per app.
//
// Steam's CheckAppOwnership is not the family-sharing source: an installed
// family game can answer "not provided" while the SteamUI still lists a real
// lender. Package 0 then makes the app look owned and the banner disappears.
//
// The client's local data carries the answer without a cross-process bridge:
//   * localconfig.vdf apptickets: app -> the package the ticket was issued
//     from (for a family-shared app, the lender's package);
//   * appmanifest_*.acf LastOwner: the real lender for apps Steam has already
//     attributed, and the package -> lender map for the rest.
// This module builds the app -> lender account id map and refreshes it when
// Steam re-reads licenses. The map is read by OwnershipHooks; it never mutates
// Steam state itself.
// ---------------------------------------------------------------------------
namespace ac::family {

// Loads the map and starts the refresh worker. Idempotent.
void Init();

// Clears the map. The refresh worker is stopped by workers::Shutdown before
// this runs.
void Shutdown();

// Asks the worker to query the family group again. Safe from any thread.
void RequestRefresh();

// True once a map has been loaded (cache or a completed refresh).
bool Ready();

// Real lender account id for appId, or 0 when the app is not family-shared.
std::uint32_t LenderFor(steam::AppId appId);

} // namespace ac::family
