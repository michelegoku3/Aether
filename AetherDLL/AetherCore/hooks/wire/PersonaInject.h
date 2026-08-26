#pragma once

#include <cstdint>

#include "core/SteamTypes.h"
#include "hooks/wire/PacketRouter.h"

// ---------------------------------------------------------------------------
// Local PersonaState template / inject for friends-UI presence.
//
// Modelled on OpenSteamTool Hooks_NetPacket_RichPresence:
//   * cache a real self-PersonaState push as a template
//   * stage a patched copy when playingAppId changes
//   * deliver by borrowing the next RecvPkt carrier
//   * re-patch periodic self pushes so the server cannot wipe us to "Online"
//
// Does not touch GetAppIDForCurrentPipe or GamesPlayed.game_id.
// ---------------------------------------------------------------------------
namespace ac::hooks::PersonaInject {

// Set the app that local presence should advertise (0 = clear). Builds a
// staged inject when a self template is already available.
// forceRestage=true rebuilds even if playingAppId is unchanged (e.g. new RP KVs).
void SetPlayingApp(steam::AppId appId, bool forceRestage = false);

steam::AppId PlayingApp();

// PersonaState recv handler. Updates the self template; if we are tracking a
// playing app (or AetherOnline persona patch is on), rewrites the body.
// Returns new body length, or -1 for no change.
std::int32_t OnPersonaStateRecv(const WireFrame& frame, std::uint8_t* out,
                                std::uint32_t outCap);

// If a staged packet is pending, temporarily replace carrier data, call
// oRecvPkt once, then restore. Safe no-op when nothing is pending.
void TryDeliver(void* recvThis, steam::CNetPacket* carrier,
                void* (*oRecvPkt)(void*, steam::CNetPacket*));

void Reset();

}  // namespace ac::hooks::PersonaInject
