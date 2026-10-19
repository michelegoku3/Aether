#pragma once

#include "hooks/wire/PacketRouter.h"
#include <cstdint>

namespace ac::hooks::AchievementModule {

// Outbound message handling (intercepts stats requests to apply spoofing)
std::int32_t HandleSendGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);
std::int32_t HandleSendClientGetUserStats(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

// Handling of inbound messages (donor data cleanup and local user data injection)
std::int32_t HandleRecvGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                                            std::uint8_t* outHdr, std::uint32_t outHdrCap, std::int32_t* outNewHdrLen);
std::int32_t HandleRecvClientGetUserStatsResponse(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

// Save handling (Store)
std::int32_t HandleSendStoreUserStats2(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap);

// Cleanly tears down any internal state. Safe to call when never started.
void Shutdown();

} // namespace ac::hooks::AchievementModule
