#pragma once

// ============================================================================
// WorkshopSync — default owner of Steam Workshop item manifests inside Steam.
//
// AetherDesk never stages Workshop manifests automatically (its monitor lane
// is disabled; the Library `Repair Workshop` button stays as a manual
// fallback). This module is the default path: it watches
// `steamapps/workshop/appworkshop_<app>.acf`, and for every subscribed item
// whose manifest is missing from depotcache it:
//   1. deletes the stale/partial `workshop/content/<app>/<item>` dir (ONLY
//      when the manifest is missing — a present manifest never touches the
//      folder), so Steam re-downloads cleanly instead of resuming corrupt
//      chunks (`50kbps then no connection` stall);
//   2. reuses the Desk Hubcap cache file when valid;
//   3. otherwise generates the manifest through the authenticated Hubcap
//      Workshop endpoint on a serialized background thread with a SHORT
//      manifest-only retry schedule (no Steam restart, no steam:// spawn;
//      Steam's own retry picks the staged manifest up).
// ============================================================================

namespace ac::workshop {

// Starts the background sync thread. No-op if already running.
void Start();

// Signals the thread to stop and joins it. Safe to call if never started.
void Stop();

}  // namespace ac::workshop
