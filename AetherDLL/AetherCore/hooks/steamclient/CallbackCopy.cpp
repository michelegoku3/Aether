#include "pch.h"
#include "hooks/steamclient/CallbackCopy.h"

#include <cstring>

#include "core/Constants.h"

namespace ac::hooks::callbackcopy {

std::optional<std::vector<std::uint8_t>> ForMaskedGame(int cb, const void* data, int size, steam::AppId realApp) {
    if (realApp == 0 || realApp == constants::kSpacewarAppId)
        return std::nullopt;
    if (!constants::achievement_cb::IsAchievementCallback(cb))
        return std::nullopt;
    // 1103 drives the overlay unlock toast, which is keyed to the real app id:
    // a masked copy would show the same toast twice.
    if (cb == constants::achievement_cb::kUserAchievementStored)
        return std::nullopt;
    if (!data || size < static_cast<int>(sizeof(std::uint64_t)))
        return std::nullopt;

    const auto* bytes = static_cast<const std::uint8_t*>(data);
    std::uint64_t gameId = 0;
    std::memcpy(&gameId, bytes, sizeof(gameId));
    if ((gameId & constants::kGameIdAppIdMask) != realApp)
        return std::nullopt;

    std::vector<std::uint8_t> copy(bytes, bytes + size);
    gameId = (gameId & ~static_cast<std::uint64_t>(constants::kGameIdAppIdMask)) |
             static_cast<std::uint64_t>(constants::kSpacewarAppId);
    std::memcpy(copy.data(), &gameId, sizeof(gameId));
    return copy;
}

} // namespace ac::hooks::callbackcopy
