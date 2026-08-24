#pragma once

#include "core/SteamTypes.h"
#include "framework.h"

#include <string>

// ---------------------------------------------------------------------------
// Localized game title lookup via steamclient CAppInfoCache.
//
// Used by the presence pipeline (game_extra_info + PersonaState game_name).
// Resolves "GetAppDataFromAppInfo" from the pattern table and captures the
// CAppInfoCache this-pointer with a one-shot MinHook detour (no VEH framework).
// Misses degrade to empty string — never fatal.
// ---------------------------------------------------------------------------
namespace ac::gamename {

// Resolve pattern + arm one-shot capture hook. Safe to call once during
// InstallAllHooks after pattern::Init and diversion load.
void Init(HMODULE diversion);

// Cached localized name for appId, or empty if AppInfo is unavailable.
std::string ForApp(steam::AppId appId);

// True once the AppInfo cache pointer has been captured.
bool Ready();

// Reverse lookup: app id whose localized name equals `name`, searched across the
// locally configured library (luadata::LibraryAppIds). 0 when there is no match.
//
// Needed by the inbound-PersonaState path: when a friend is masked as Spacewar
// the CM still ships the real title in Friend.game_name, so the viewer can
// recover the real app id -- and therefore the real icon -- without the server
// ever transmitting it. Case-insensitive, results cached both ways.
steam::AppId ResolveAppIdByName(const std::string& name);

}  // namespace ac::gamename
