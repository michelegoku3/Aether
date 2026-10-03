#pragma once

#include <cstdint>
#include <memory>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// PresenceSession — the AetherOnline/ShowOnline masquerade, owned by ONE
// module (hooks/steamclient/AetherOnlineHooks.cpp) and read by everyone
// else through an immutable snapshot.
//
// Replaces the three scattered atomics that used to live in AetherCoreState
// (aetherOnlineRealAppId / showOnlineAppId / spacewarSpoofExpected). The old
// design was a distributed state machine: 5 modules communicated through
// globals, stores happened one field at a time (torn reads possible) and the
// state stayed stale between game exit and the next SpawnProcess.
//
// Invariants now:
//   * Publish() is the ONLY writer (called from h_SpawnProcess) and stores
//     all fields together — readers can never observe a half-updated state.
//   * ClearShowOnline() is the single narrow exception (foreign-crack 480
//     session takes over: GamesPlayedModule clears only the showonline bit).
//   * EndSession() resets everything when Steam reports zero running games,
//     so no stale identity survives after the game process exits.
//   * Readers take Current() (shared_ptr<const SessionSnapshot>, never null)
//     or the hot-path accessors (plain atomic loads, zero allocations).
// ---------------------------------------------------------------------------
namespace ac::presence {

struct SessionSnapshot {
    steam::AppId realAppId = 0;         // AetherOnline: real app behind the 480 mask
    steam::AppId showOnlineAppId = 0;   // ShowOnline: wire-only presence rewrite
    bool spacewarSpoofExpected = false; // UCO2/OFME spoof on disk at spawn time

    // "aetheronline" | "showonline" | "none" (diagnostics only).
    const char* ModeText() const {
        if (realAppId != 0) return "aetheronline";
        if (showOnlineAppId != 0) return "showonline";
        return "none";
    }
};

// Owner: AetherOnlineHooks::h_SpawnProcess. Publishes the full session state
// atomically and logs the transition (one INFO per launch decision).
void Publish(SessionSnapshot next);

// GamesPlayedModule only: a foreign crack (UCO2/OFME) owns the 480 session,
// so the showonline rewrite must stop. No-op when already zero.
void ClearShowOnline();

// GamesPlayedModule only: Steam just reported zero running games — the
// masquerade (if any) is over. Resets to the empty session and logs it.
void EndSession();

// Immutable snapshot; never null. Cheap: one mutex + shared_ptr copy.
std::shared_ptr<const SessionSnapshot> Current();

// Hot-path accessors (atomic loads, no allocation, no lock):
steam::AppId RealAppId();
steam::AppId ShowOnlineAppId();
bool SpacewarSpoofExpected();

}  // namespace ac::presence
