#pragma once

// ---------------------------------------------------------------------------
// SteamUI app-overview hooks.
//
// One pass-through hook (`FillInAppOverview`) observes every card the client
// builds and, once the compiled layout has been confirmed by two distinct apps
// and Aether already answers the app as owned, sets its ownership flag.
//
// The write happens on the client's own callback thread; nothing here queues
// work or touches the object from a worker. Visibility, size, state and
// achievements are not handled by this module.
// ---------------------------------------------------------------------------
namespace ac::hooks::steamui {

// Registers the hook on the live steamui module. Returns false (and records a
// pattern miss) when the address cannot be resolved; the rest of the client
// keeps working.
bool InstallAppOverviewHooks();

bool AppOverviewHooksInstalled();

} // namespace ac::hooks::steamui
