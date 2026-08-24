#pragma once

#include "framework.h"

#include "core/SteamTypes.h"

namespace ac::hooks {

// Registers OnlineFix hooks on the diverted steamclient: SpawnProcess (mask the
// real app as Spacewar/480 when launched with -onlinefix, or record a
// wire-only -showonline session WITHOUT masking) and GetAppIDForCurrentPipe
// (translate 480 back to the real app id during OnlineFix stats scopes).
void RegisterOnlineFixHooks(HMODULE diversion);

// Calls the original (un-hooked) GetAppIDForCurrentPipe with the captured
// SteamEngine pointer. Returns 0 if the engine has not been captured yet or the
// trampoline is unavailable. Exposed for the IPC capture layer so it doesn't
// re-resolve a function this module already owns.
steam::AppId CallOriginalGetAppIdForCurrentPipe();

}  // namespace ac::hooks
