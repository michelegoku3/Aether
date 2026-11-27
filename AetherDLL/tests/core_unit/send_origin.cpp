#include "cases.h"

#include "hooks/wire/SendOriginPolicy.h"

void SendOriginPolicyTest() {
    using ac::hooks::ExtractReservedBufferSlice;
    using ac::hooks::IsAuthenticatedCmOrigin;
    using ac::hooks::IsServiceCallOrigin;
    using ac::hooks::ShouldFallbackToWebSocket;
    using ac::hooks::ShouldSkipRawSendHint;

    // WebUI / pre-login frames have steamid=0 even with a transport handle: must be rejected.
    CHECK(!IsAuthenticatedCmOrigin(0, true));
    CHECK(!IsAuthenticatedCmOrigin(76561198000000000ULL, false));
    CHECK(IsAuthenticatedCmOrigin(76561198000000000ULL, true));

    // Service origin requires both service eMsg (151 or 146) and authenticated CM identity.
    CHECK(!IsServiceCallOrigin(151, 0, true));
    CHECK(!IsServiceCallOrigin(146, 0, true));
    CHECK(!IsServiceCallOrigin(717, 76561198000000000ULL, true));
    CHECK(IsServiceCallOrigin(151, 76561198000000000ULL, true));
    CHECK(IsServiceCallOrigin(146, 76561198000000000ULL, true));

    // Raw CNet internal HTTP / WebSocket / transport hints must be skipped cleanly.
    CHECK(ShouldSkipRawSendHint(0x1A92u));
    CHECK(ShouldSkipRawSendHint(0x1A93u));
    CHECK(ShouldSkipRawSendHint(0x1A9Eu));
    CHECK(!ShouldSkipRawSendHint(151u));
    CHECK(!ShouldSkipRawSendHint(8903u));

    // Reserved-buffer slicing guards against null, negative reserved offsets, and empty slices.
    const std::uint8_t backing[16] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15};
    const std::uint8_t* slicePtr = nullptr;
    std::uint32_t sliceLen = 0;

    CHECK(!ExtractReservedBufferSlice(nullptr, 16, 4, slicePtr, sliceLen));
    CHECK(!ExtractReservedBufferSlice(backing, 16, -1, slicePtr, sliceLen));
    CHECK(!ExtractReservedBufferSlice(backing, 4, 4, slicePtr, sliceLen));
    CHECK(!ExtractReservedBufferSlice(backing, 3, 4, slicePtr, sliceLen));
    CHECK(ExtractReservedBufferSlice(backing, 16, 4, slicePtr, sliceLen));
    CHECK_EQ(slicePtr, backing + 4);
    CHECK_EQ(sliceLen, std::uint32_t{12});

    // Only fall back to hooking WebSocket framing when either CNet entry point is missing.
    CHECK(!ShouldFallbackToWebSocket(true, true));
    CHECK(ShouldFallbackToWebSocket(true, false));
    CHECK(ShouldFallbackToWebSocket(false, true));
    CHECK(ShouldFallbackToWebSocket(false, false));
}
