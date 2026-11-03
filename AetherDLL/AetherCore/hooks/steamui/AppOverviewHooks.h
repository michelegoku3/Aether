#pragma once

#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// SteamUI app-overview hooks.
//
// One pass-through hook (`FillInAppOverview`) observes every card the client
// builds and, once the compiled layout has been confirmed by two distinct apps
// and Aether already answers the app as owned, sets its ownership flag.
//
// A second pair carries a version change back to the client: `MarkAppChange` is
// observed only to learn the object that owns the app list, and the frame
// callback drains the ids a Lua change queued and asks that owner to re-evaluate
// them (RequestAppRecheck). A moved pin means nothing until the client looks
// again, and only its own thread may make it look.
//
// All of it happens on the client's own callback threads; nothing here touches
// an object from a worker. Visibility, size, state and achievements are not
// handled by this module.
// ---------------------------------------------------------------------------
namespace ac::hooks::steamui {

// Registers the card hook and the recheck pair on the live steamui module.
// Returns false (and records a pattern miss) when the card hook cannot be
// resolved, so the batch that called it stays retryable; a missing recheck
// pattern disables only that pair.
bool InstallAppOverviewHooks();

bool AppOverviewHooksInstalled();

// Queues apps whose content intent changed, so the client re-evaluates them.
//
// The ids are drained by the client's frame callback, which is the only place
// MarkAppChange may be called: the caller (the worker that applied the Lua)
// never touches a Steam object and never needs the app-change owner. An id
// already queued is ignored, because it is going to be rechecked anyway; a full
// queue refuses the request and says so in the log, since nothing re-offers it
// until the next apply.
void RequestAppRecheck(const std::vector<steam::AppId>& ids);

// False when the module never installed the MarkAppChange/frame pair: the ids
// still queue, but nothing will ask the client to look again.
bool AppRecheckSupported();

} // namespace ac::hooks::steamui
