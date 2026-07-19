#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// ---------------------------------------------------------------------------
// Unified GamesPlayed send path (eMsg 742 / 5410).
//
// Always (settings permitting):
//   * learn self SteamID from the protobuf header
//   * track topmost app for local PersonaInject (Lua-managed, non-owned)
//   * fill game_extra_info with the display name when known
//
// OnlineFix: game_id stays 480; extra_info carries the real title.
// No-OnlineFix: game_id stays the real appid; extra_info is cosmetic polish;
//   local inject is what fixes generic "Online" on the local client.
//
// NEVER rewrites game_id to real under OnlineFix (session identity).
// ---------------------------------------------------------------------------
namespace ac::hooks::GamesPlayed {

// Returns rewritten body length, or -1 if the wire body is unchanged.
// Tracking / inject side effects may still run when returning -1.
std::int32_t HandleSend(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

// RichPresenceUpload (eMsg 7501): capture KV blob for the playing app.
// Always returns -1 (never rewrites the upload).
std::int32_t HandleRichPresenceUpload(const WireFrame& frame);

}  // namespace ac::hooks::GamesPlayed
