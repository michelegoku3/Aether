#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// Authenticated manifest bridge. The request-code message is used only as the
// synchronization point: ManifestFetch obtains and validates the exact local
// or Hubcap manifest, then the response is adapted back to Steam's expected
// request-code shape.
namespace ac::hooks::ManifestBridge {

std::int32_t HandleSend(const WireFrame& frame);

std::int32_t HandleRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                        std::uint8_t* outHeader, std::uint32_t outHeaderCap,
                        std::int32_t* outHeaderLen);

}  // namespace ac::hooks::ManifestBridge
