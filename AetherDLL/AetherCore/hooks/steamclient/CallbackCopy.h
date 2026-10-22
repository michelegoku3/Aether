#pragma once

#include <cstdint>
#include <optional>
#include <vector>

#include "core/SteamTypes.h"

namespace ac::hooks::callbackcopy {

// Returns a private copy of an achievement callback whose m_nGameID is
// rewritten from `realApp` to the Spacewar/480 mask, or nullopt when the
// callback needs no masked copy: not an achievement event, UserAchievementStored
// (its 480 copy would duplicate the overlay toast), too short, or already
// masked.
//
// The caller's buffer is never modified. Steam receives this copy for the
// second dispatch; the first send keeps the original bytes, so a deferred
// delivery can never observe a rewritten payload.
std::optional<std::vector<std::uint8_t>> ForMaskedGame(int cb, const void* data, int size, steam::AppId realApp);

} // namespace ac::hooks::callbackcopy
