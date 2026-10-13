#pragma once

// ============================================================================
// PlaytimeMirror — per-account playtime backup.
//
// Source: userdata\<account>\config\localconfig.vdf (Software/Valve/Steam/"Apps"
// section, MINUTE values: Playtime, Playtime2wks, PlaytimeDisconnected,
// LastPlayed + autocloud {lastlaunch,lastexit}).
// Client-side data for managed games: same fragility as achievements.
//
// Output: <AetherData>\backup\playtime\UserPlaytime_<account>.json with
// MONOTONE merge (per field, the historic max wins).
// ============================================================================
namespace ac::backup::playtime {

// Re-reads the localconfig of EVERY userdata\* folder found and refreshes the
// respective snapshots. Best-effort: errors only logged.
void RefreshAllAccounts();

} // namespace ac::backup::playtime
