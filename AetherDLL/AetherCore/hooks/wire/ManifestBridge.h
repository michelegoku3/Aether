#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// Archived reference only. ManifestBridge.cpp is intentionally excluded from
// the production AetherCore target and PacketRouter never dispatches this
// request-code path. Keep the implementation documented for future, explicitly
// authenticated pipelines; do not re-enable it as an automatic update or
// Workshop source without a reviewed provider design.
namespace ac::hooks::ManifestBridge {

std::int32_t HandleSend(const WireFrame& frame);

std::int32_t HandleRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                        std::uint8_t* outHeader, std::uint32_t outHeaderCap,
                        std::int32_t* outHeaderLen);

}  // namespace ac::hooks::ManifestBridge
