#pragma once

#include <cstdint>

// Family-sharing lock prevention. Steam can revoke access to injected ("shared")
// apps when the real owner starts playing; clearing these notification bodies
// keeps fake-owned apps playable.
namespace ac::hooks::FamilySharing {

// Signals the router to drop the body of a lock/stop notification. Returns 0
// (the router interprets 0 as "clear body").
std::int32_t ClearBody();

}  // namespace ac::hooks::FamilySharing
