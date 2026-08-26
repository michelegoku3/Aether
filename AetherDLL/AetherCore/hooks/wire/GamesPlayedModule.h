#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// ---------------------------------------------------------------------------
// Unified GamesPlayed send path (eMsg 742 / 5410).
//
// Show Online: rewrite real appid -> 480 + extra name + hidden appid
//   (Aether friends reconstruct; vanilla see Spacewar + extra name).
// UCO2 / OFME / Online Aether: game_id stays 480; extra_info = real name
//   only. No hidden appid, no self-inject as the real game (that kills
//   the Spacewar lobby / invites).
// ---------------------------------------------------------------------------
namespace ac::hooks::GamesPlayed {

// Returns rewritten body length, or -1 if the wire body is unchanged.
// Tracking / inject side effects may still run when returning -1.
std::int32_t HandleSend(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

// RichPresenceUpload (eMsg 7501): capture KV blob for the playing app.
// Always returns -1 (never rewrites the upload).
std::int32_t HandleRichPresenceUpload(const WireFrame& frame);

}  // namespace ac::hooks::GamesPlayed
