#pragma once

#include "framework.h"
#include "core/SteamTypes.h"

#include <cstdint>
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
// InstallAllHooks after pattern::Init and steamclient load.
void Init(HMODULE steamclient);

// Cached localized name for appId, or empty if AppInfo is unavailable.
std::string ForApp(steam::AppId appId);

// True once the AppInfo cache pointer has been captured.
bool Ready();

// True when the client's cached AppInfo for `appId` lists a public manifest for
// `depotId`, i.e. the depot is part of that app's content. Steam plans a
// download only from those depots; a plan row for any other depot makes the
// client reject the table.
bool HasPublicDepot(steam::AppId appId, steam::AppId depotId);

// Size in bytes of that depot's public manifest, or 0 when the AppInfo does not
// carry it.
std::uint64_t PublicDepotSize(steam::AppId appId, steam::AppId depotId);

// gid of that depot's current public manifest, or 0 when the AppInfo does not
// carry it.
std::uint64_t PublicDepotGid(steam::AppId appId, steam::AppId depotId);

// Reverse lookup: app id whose localized name equals `name`, searched across the
// locally configured library (luadata::LibraryAppIds). 0 when there is no match.
//
// Needed by the inbound-PersonaState path: when a friend is masked as Spacewar
// the CM still ships the real title in Friend.game_name, so the viewer can
// recover the real app id -- and therefore the real icon -- without the server
// ever transmitting it. Case-insensitive, results cached both ways.
steam::AppId ResolveAppIdByName(const std::string& name);

} // namespace ac::gamename
