#include "pch.h"
#include "utils/Memory.h"

#include <cstdint>

namespace ac::utils::memory {

bool IsWritableRange(const void* base, std::size_t size) {
    if (!base || size == 0)
        return false;

    const auto* cursor = static_cast<const std::uint8_t*>(base);
    std::size_t remaining = size;
    while (remaining > 0) {
        MEMORY_BASIC_INFORMATION info{};
        if (VirtualQuery(cursor, &info, sizeof(info)) != sizeof(info))
            return false;
        if (info.State != MEM_COMMIT)
            return false;
        const DWORD allowed = PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
        if ((info.Protect & allowed) == 0)
            return false;
        // PAGE_GUARD pages fault even when the base protection allows writes.
        if ((info.Protect & (PAGE_GUARD | PAGE_NOACCESS)) != 0)
            return false;

        const auto* regionEnd = static_cast<const std::uint8_t*>(info.BaseAddress) + info.RegionSize;
        if (regionEnd <= cursor)
            return false;
        const auto available = static_cast<std::size_t>(regionEnd - cursor);
        if (available >= remaining)
            return true;
        remaining -= available;
        cursor = regionEnd;
    }
    return true;
}

} // namespace ac::utils::memory
