#include "cases.h"

#include <algorithm>
#include <cstring>

#include "core/Constants.h"
#include "hooks/steamclient/CallbackCopy.h"

// A deferred Steam callback keeps the pointer it was given and may deliver it
// on another thread. The masked copy therefore has to be a private buffer: the
// original bytes must stay untouched for the first (real app id) delivery.
void CallbackCopyTest() {
    using namespace ac::hooks::callbackcopy;
    using namespace ac::constants;
    constexpr ac::steam::AppId kReal = 1144200;

    const auto make = [](std::uint32_t appId) {
        std::vector<std::uint8_t> bytes(24, 0xAB);
        const std::uint64_t gameId = static_cast<std::uint64_t>(appId) | (1ull << 32);
        std::memcpy(bytes.data(), &gameId, sizeof(gameId));
        return bytes;
    };
    const auto original = make(kReal);

    const auto copy =
        ForMaskedGame(achievement_cb::kUserStatsReceived, original.data(), static_cast<int>(original.size()), kReal);
    CHECK(copy.has_value());
    CHECK_EQ(copy->size(), original.size());

    std::uint64_t masked = 0;
    std::memcpy(&masked, copy->data(), sizeof(masked));
    CHECK_EQ(static_cast<std::uint32_t>(masked & kGameIdAppIdMask), kSpacewarAppId);
    CHECK_EQ(masked >> 32, 1ull);
    // Everything after m_nGameID is copied verbatim.
    CHECK(std::equal(copy->begin() + 8, copy->end(), original.begin() + 8));
    // The caller's buffer was not touched.
    CHECK(original == make(kReal));

    // 1103 is delivered once, with the real app id: a masked copy duplicates
    // the overlay toast.
    CHECK(!ForMaskedGame(achievement_cb::kUserAchievementStored, original.data(), static_cast<int>(original.size()),
                         kReal));
    // Not an achievement callback.
    CHECK(!ForMaskedGame(9999, original.data(), static_cast<int>(original.size()), kReal));
    // Too short for m_nGameID.
    CHECK(!ForMaskedGame(achievement_cb::kUserStatsReceived, original.data(), 4, kReal));
    // Already masked, or no session: nothing to rewrite.
    const auto maskedSource = make(kSpacewarAppId);
    CHECK(!ForMaskedGame(achievement_cb::kUserStatsReceived, maskedSource.data(), static_cast<int>(maskedSource.size()),
                         kReal));
    CHECK(!ForMaskedGame(achievement_cb::kUserStatsReceived, original.data(), static_cast<int>(original.size()), 0));
}
