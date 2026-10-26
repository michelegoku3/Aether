#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// Manifest request-code bridge. Steam asks the CM for a request code before it
// downloads a depot it does not consider owned; when the exact manifest is
// already local, the honest answer is code 0. The bridge only rewrites a reply
// it correlated with a request it saw, and only once the manifest is on disk:
// it never waits, so Steam's network thread is never blocked.
namespace ac::hooks::ManifestBridge {

std::int32_t HandleSend(const WireFrame& frame);

std::int32_t HandleRecv(const WireFrame& frame, std::uint64_t jobIdTarget, std::uint8_t* out, std::uint32_t outCap,
                        std::uint8_t* outHeader, std::uint32_t outHeaderCap, std::int32_t* outHeaderLen);

} // namespace ac::hooks::ManifestBridge
