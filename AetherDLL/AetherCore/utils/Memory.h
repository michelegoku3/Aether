#pragma once

#include <cstddef>

namespace ac::utils::memory {

// True when every page of [base, base + size) is committed and writable: the
// whole range can be written without faulting. Guard and no-access pages make
// it false, and so does a range that crosses into an uncommitted region.
//
// Used to verify that a hook-visible grow helper actually made the requested
// bytes available before a caller trusts a length field.
bool IsWritableRange(const void* base, std::size_t size);

} // namespace ac::utils::memory
