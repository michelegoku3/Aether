#pragma once

#include "framework.h"

#include "core/SteamTypes.h"

namespace ac::hooks {

// Registers the AetherOnline hooks on the diverted steamclient: SpawnProcess
// (mask the real app as Spacewar/480 when launched with -aetheronline, or record
// a wire-only -showonline session WITHOUT masking), GetAppIDForCurrentPipe
// (translate 480 back to the real app id during AetherOnline stats scopes) and
// BuildSpawnEnvBlock (patch the overlay CGameID from 480 to the real app id).
void RegisterAetherOnlineHooks(HMODULE diversion);

// Calls the original (un-hooked) GetAppIDForCurrentPipe with the captured
// SteamEngine pointer. Returns 0 if the engine has not been captured yet or the
// trampoline is unavailable. Exposed for the IPC capture layer so it doesn't
// re-resolve a function this module already owns.
steam::AppId CallOriginalGetAppIdForCurrentPipe();

} // namespace ac::hooks
