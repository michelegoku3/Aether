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

}  // namespace ac::hooks
