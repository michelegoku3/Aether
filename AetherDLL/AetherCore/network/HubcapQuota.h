#pragma once

// ============================================================================
// HubcapQuota — the AetherDLL side of the shared daily generation budget.
//
// AetherDesk and AetherDLL both consume Hubcap's generation endpoints (the
// Desk for downloads and version switches, the DLL for on-demand runtime
// lookups). The local daily budget therefore lives in ONE file —
// <AetherData>\state\hubcap_generation_quota.json — and every
// read-modify-write cycle from either process runs while holding the sibling
// .lock file (exclusive create; the holder's handle deletes the file on
// close, so a crash can never leak the lock; an abandoned lock is broken
// after 5 s). The Rust twin of this protocol is
// AetherDesk/src-tauri/src/providers/hubcap_generation.rs.
// ============================================================================

namespace ac::hubcapquota {

// Attempts to reserve one game-manifest generation from the shared daily
// budget (1500/day, reset at fixed midnight EST). Returns false when the
// budget is exhausted or the shared state is unavailable in a way that must
// block generation.
bool TryReserveGameGeneration();

// Returns one reserved unit after a failed generation (best effort).
void ReleaseGameGeneration();

// Attempts to reserve one Workshop-manifest generation from the shared daily
// budget (500/day, reset at fixed midnight EST). Workshop sync is owned by
// AetherDLL by default (AetherDesk's automatic lane is disabled); the manual
// Desk Repair Workshop button spends from the same shared file.
bool TryReserveWorkshopGeneration();

// Returns one reserved Workshop unit after a failed generation (best effort).
void ReleaseWorkshopGeneration();

}  // namespace ac::hubcapquota
