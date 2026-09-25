#pragma once

#include "framework.h"

namespace ac::hooks {

// In copy mode, arm steamui!LoadModuleWithPath as soon as pattern resolution
// completes, before IPC and Lua initialization can delay it. If steamui is
// missing, a bounded retry runs in the background. No-op in live mode.
void ArmSteamUiRedirectEarly();

// Installs the steamclient hook batch; idempotently tries the UI redirect again
// (also used by the late-pattern retry).
void InstallAllHooks();

// Stops and joins the deferred steamui retry thread. Safe to call when the
// retry never started or already finished. Called from dllmain::Shutdown.
void ShutdownSteamUiRetry();

// Starts the background late-pattern retry (no-op when every pattern table is
// already available at init). Re-probes the pattern sources for a bounded
// window and re-runs the hook batch in-session as soon as a previously-missing
// table appears, so the hooks install without a Steam restart.
void StartPatternLateRetry();

// Stops and joins the late-pattern retry thread. Safe when never started.
void StopPatternLateRetry();

}  // namespace ac::hooks
