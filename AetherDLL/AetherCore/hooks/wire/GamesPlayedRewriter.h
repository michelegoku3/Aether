#pragma once

#include <cstdint>

#include "core/Settings.h"
#include "core/SteamTypes.h"

class CMsgClientGamesPlayed;

// ---------------------------------------------------------------------------
// Wire-level games_played rewriter (P9 extraction from GamesPlayedModule).
//
// Rewrite() runs BOTH presence passes over the message, in the historical
// order, with identical gating and logging:
//   1. -showonline presence rewrite (real appid entry -> masked 480 entry);
//   2. game_extra_info annotation (foreign 480 spoof / managed apps).
// It is deterministic in (msg, ctx): no global state besides logging
// deduplication, so the handler (GamesPlayedModule) stays pure glue.
// ---------------------------------------------------------------------------
namespace ac::hooks::GamesPlayedRewriter {

struct RewriteContext {
    const Settings& settings;
    steam::AppId soSession = 0;    // 0 = no -showonline session to rewrite
    bool foreignSpoof = false;     // UCO2/OFME owns the 480 session
    steam::AppId spoofReal = 0;    // real app behind the foreign 480
};

// Returns true when the message was modified and must be re-serialized.
bool Rewrite(CMsgClientGamesPlayed& msg, const RewriteContext& ctx);

}  // namespace ac::hooks::GamesPlayedRewriter
