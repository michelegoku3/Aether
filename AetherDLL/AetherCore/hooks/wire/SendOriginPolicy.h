#pragma once

#include <cstdint>

#include "core/Constants.h"

namespace ac::hooks {

// Pure transport/origin decision helpers for wire-level send interception.
// Kept inline without Win32 or protobuf headers so unit tests validate every
// rule directly.

// Raw CNet::BAsyncSend hints emitted by lower framing layers (HTTP, WebSocket,
// transport control). Packets using these hints carry transport framing rather
// than a raw Steam MsgHdr and must not be parsed as CM frames.
inline constexpr std::uint32_t kCNetHintHttp = 0x1A92u;
inline constexpr std::uint32_t kCNetHintWebSocket = 0x1A93u;
inline constexpr std::uint32_t kCNetHintTransportCtrl = 0x1A9Eu;

inline bool IsAuthenticatedCmOrigin(std::uint64_t steamId, bool hasTransportHandle) {
    return steamId != 0 && hasTransportHandle;
}

inline bool IsServiceCallOrigin(std::uint32_t eMsg, std::uint64_t steamId, bool hasTransportHandle) {
    const bool isServiceMsg = (eMsg == ac::constants::emsg::kServiceMethodCallFromClient ||
                               eMsg == ac::constants::emsg::kServiceMethodCallFromClientAlt);
    return isServiceMsg && IsAuthenticatedCmOrigin(steamId, hasTransportHandle);
}

inline bool ShouldSkipRawSendHint(std::uint32_t eMsgHint) {
    return eMsgHint == kCNetHintHttp || eMsgHint == kCNetHintWebSocket || eMsgHint == kCNetHintTransportCtrl;
}

// Extracts the active [base + cubReserved, base + tellPut) payload slice from
// a CUtlBuffer passed to CNet::BAsyncSendSwapReserved.
inline bool ExtractReservedBufferSlice(const std::uint8_t* base, std::int32_t tellPut, std::int32_t cubReserved,
                                       const std::uint8_t*& outData, std::uint32_t& outLen) {
    outData = nullptr;
    outLen = 0;
    if (!base || cubReserved < 0 || tellPut <= cubReserved) {
        return false;
    }
    outData = base + cubReserved;
    outLen = static_cast<std::uint32_t>(tellPut - cubReserved);
    return true;
}

// Unified CNet hooks cover UDP, TCP, and WebSocket transports. Only fall back
// to hooking CWebSocketConnection::BBuildAndAsyncSendFrame when either CNet hook
// is unavailable.
inline bool ShouldFallbackToWebSocket(bool cnetSendHooked, bool cnetSwapHooked) {
    return !(cnetSendHooked && cnetSwapHooked);
}

} // namespace ac::hooks
