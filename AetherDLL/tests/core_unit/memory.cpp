#include "cases.h"

#include <cstdint>

#include "utils/Memory.h"

// The reply-buffer grow is verified against the real pages: a committed range
// is writable, a read-only or guard page is not, and a range that runs into an
// uncommitted region is refused.
void MemoryRangeTest() {
    using ac::utils::memory::IsWritableRange;

    std::vector<std::uint8_t> buffer(4096, 0);
    CHECK(IsWritableRange(buffer.data(), buffer.size()));
    CHECK(!IsWritableRange(nullptr, 16));
    CHECK(!IsWritableRange(buffer.data(), 0));

    SYSTEM_INFO info{};
    GetSystemInfo(&info);
    const std::size_t page = info.dwPageSize;
    auto* region = static_cast<std::uint8_t*>(VirtualAlloc(nullptr, page * 2, MEM_RESERVE, PAGE_NOACCESS));
    CHECK(region != nullptr);
    CHECK(VirtualAlloc(region, page, MEM_COMMIT, PAGE_READWRITE) == region);
    CHECK(IsWritableRange(region, page));
    // One byte past the committed page lands in the reserved (no-access) page.
    CHECK(!IsWritableRange(region, page + 1));

    CHECK(VirtualAlloc(region + page, page, MEM_COMMIT, PAGE_READONLY) == region + page);
    CHECK(!IsWritableRange(region + page, 1));

    DWORD previous = 0;
    CHECK(VirtualProtect(region, page, PAGE_READWRITE | PAGE_GUARD, &previous) != 0);
    CHECK(!IsWritableRange(region, 1));

    CHECK(VirtualFree(region, 0, MEM_RELEASE) != 0);
}
