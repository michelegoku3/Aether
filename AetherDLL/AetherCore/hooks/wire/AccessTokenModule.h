#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// Outgoing PICS product-info requests: inject the configured access token for
// any app that has one, so Steam returns restricted product info.
namespace ac::hooks::AccessToken {

// Returns the new serialized body length written to out (<= outCap), or -1 if
// no app in the request needs a token (frame left unchanged).
std::int32_t HandleSend(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

}  // namespace ac::hooks::AccessToken
