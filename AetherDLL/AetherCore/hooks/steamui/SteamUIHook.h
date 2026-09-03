#pragma once

#include "framework.h"

namespace ac::hooks {

// Installs the steamclient hook batch immediately (does NOT block on
// steamui.dll), then starts a deferred retry thread that installs the
// LoadModuleWithPath redirect (steamui.dll) as soon as the module appears,
// within a bounded budget. Publishes hook status after each batch. Intended
// to run on the init thread.
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
