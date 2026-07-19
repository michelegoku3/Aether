#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

namespace ac::hooks::EticketModule {

std::int32_t HandleRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

}  // namespace ac::hooks::EticketModule
