#pragma once

#include "framework.h"

// ---------------------------------------------------------------------------
// SteamUIHook — ONLY the steamclient64.dll load redirect (P8).
//
// Orchestration (steamclient batch, deferred retry, late-pattern retries)
// lives in core/HookBootstrap. This module does exactly one thing: in copy
// mode, when steamui.dll loads steamclient64.dll, the call is diverted to
// acoverlay.dll; in live mode it is a no-op.
// ---------------------------------------------------------------------------
namespace ac::hooks::steamui {

// Installs the hook on steamui!LoadModuleWithPath. Returns true only when the
// redirect is actually armed or unneeded (live mode): a mapped steamui DLL is
// NOT enough (missing pattern or failed MinHook keep the bootstrap retry
// alive). Thread-safe: takes the bootstrap batch mutex because it may race the
// batch re-run.
bool InstallSteamUiRedirect();

} // namespace ac::hooks::steamui
