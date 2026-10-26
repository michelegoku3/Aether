#pragma once

#include <cstdint>

#include "core/AetherCoreState.h"
#include "core/SteamTypes.h"

namespace ac::steamvec {

// Grows a Steam vector through steamclient's CUtlMemory<T>::Grow, published in
// g_state because the license path resolves it once for package 0 and the
// depot plan needs it for a different element type. CUtlMemory is the first
// member of CUtlVector, so the vector's address is the memory header's address.
// The grow helper returns void: callers re-check the vector with its guard
// instead of trusting a return value.
template <class T> bool Reserve(steam::CUtlVector<T>& vec, std::uint32_t needed) {
    if (needed <= vec.mem.allocationCount)
        return true;
    auto grow = reinterpret_cast<void (*)(void*, int)>(g_state.pCUtlMemoryGrow.load());
    if (!grow)
        return false;
    grow(&vec, static_cast<int>(needed - vec.size));
    return true;
}

} // namespace ac::steamvec
