#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// ---------------------------------------------------------------------------
// PersonaState recv facade.
//
// All real work lives in PersonaInject (template, self patch, OF friend patch).
// Kept as a named module so PacketRouter dispatch stays readable.
// ---------------------------------------------------------------------------
namespace ac::hooks::RichPresence {

std::int32_t HandleRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

}  // namespace ac::hooks::RichPresence
