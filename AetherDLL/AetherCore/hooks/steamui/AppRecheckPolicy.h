#pragma once

#include <cstddef>
#include <cstdint>
#include <deque>
#include <unordered_set>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// App recheck policy: which apps must be asked to re-evaluate their content,
// with no logger, no shared state and no Windows calls, so the unit suite
// exercises the real thing.
//
// A version apply rewrites the Lua and nothing else. The client may keep the
// plan it built before the reload, so an app whose pins moved keeps showing the
// build it already has until something asks it to look again. This layer names
// the apps that request is about, and carries them to the SteamUI thread without
// repeats.
// ---------------------------------------------------------------------------
namespace ac::hooks::steamui::policy {

// One depot pin of one app: the manifest id the script asks Steam to install.
// Aggregate on purpose: the decision is read from plain tables, not from a
// class that could drift away from what the Lua actually says.
struct PinRow {
    steam::AppId depot;
    std::uint64_t gid;

    bool operator==(const PinRow& other) const noexcept { return depot == other.depot && gid == other.gid; }
};

// Depot keyed. Order and duplicates are decided by SelectRecheck, so a caller
// can hand over the rows in whatever order the script produced them.
using PinSet = std::vector<PinRow>;

// The pins one app carried before and after a Lua reload.
struct PinChange {
    steam::AppId app;
    PinSet before;
    PinSet after;
};

// Apps whose content intent changed: sorted, and each app named once.
//
// Both sides are normalized first (sorted, then deduplicated by depot) because
// the Lua writer has no ordering contract. A reload that only reordered rows
// did not change what the app should install, and asking Steam to rebuild the
// app for it would be a visible refresh for nothing.
std::vector<steam::AppId> SelectRecheck(const std::vector<PinChange>& changes);

// The apps waiting for a client recheck, carried from the worker that applied
// the Lua change to the SteamUI thread that can act on it.
//
// Bounded, and a full queue refuses the push instead of growing or evicting:
// the entries already accepted are the reloads that have been reported, so
// dropping one of them loses that app's update with nothing left to re-offer
// it. The Lua edit that filled the queue is the only thing that would.
//
// Not thread-safe: the owner selects the lock, since the writer is the apply
// worker and the reader is the SteamUI thread.
class RecheckQueue {
  public:
    static constexpr std::size_t kCapacity = 64;

    // False for two different reasons, which the caller must tell apart.
    // Already queued: the app will still be rechecked, so nothing is lost.
    // Rejected: the queue is full, so nothing was added for this call. A refused
    // id stays unrechecked until that app's Lua changes again — that refusal has
    // to be reported, not read as "later". A full queue also refuses an app it
    // already holds, which the capacity check answers before the duplicate check
    // does.
    bool Push(steam::AppId app);

    // Oldest first, so an app edited early is not starved by a burst of edits.
    // False when the queue is empty.
    bool Pop(steam::AppId& app);

    std::size_t Size() const;

  private:
    std::deque<steam::AppId> entries_;
    std::unordered_set<steam::AppId> inFlight_;
};

} // namespace ac::hooks::steamui::policy
