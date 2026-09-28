#include "pch.h"
#include "core/StructGuard.h"

#include <atomic>

#include "core/Logger.h"

namespace ac::abi::guard {
namespace {
constexpr const char* kModule = "Abi.StructGuard";
std::atomic<std::uint32_t> g_rejections{0};
}  // namespace

const char* ReasonText(Reason r) {
    switch (r) {
    case Reason::Ok: return "ok";
    case Reason::NullObject: return "null object";
    case Reason::BoolBlockNotBoolean: return "bool block holds non-boolean bytes (layout shifted?)";
    case Reason::ReleaseStateOutOfRange: return "releaseState out of range";
    case Reason::PackageCountAbsurd: return "existInPackageNums absurd";
    case Reason::VectorSizeExceedsAlloc: return "vector size exceeds allocationCount";
    case Reason::VectorMemoryNull: return "vector has elements but null storage";
    case Reason::VectorMemoryMisaligned: return "vector storage is misaligned";
    case Reason::VectorAllocAbsurd: return "vector allocationCount absurd";
    }
    return "unknown";
}

void CountRejection(const char* what, Reason r) {
    g_rejections.fetch_add(1, std::memory_order_relaxed);
    // ERROR: this means we refused to write into a Steam struct because it did
    // not look like the struct we compiled against. Either the layout moved
    // (Valve update) or we were handed the wrong pointer. Both need a human.
    AC_LOG_ERROR(kModule, "%s rejected: %s. Write skipped — the feature degrades, Steam does not.",
                 what, ReasonText(r));
    diag::Record("abi_struct_guard", std::string(what) + ": " + ReasonText(r));
}

std::uint32_t RejectionCount() { return g_rejections.load(std::memory_order_relaxed); }

void ResetCounters() { g_rejections.store(0, std::memory_order_relaxed); }

}  // namespace ac::abi::guard
