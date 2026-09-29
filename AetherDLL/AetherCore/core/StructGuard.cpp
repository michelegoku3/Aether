#include "pch.h"
#include "core/StructGuard.h"

#include <atomic>

#include "core/Logger.h"

namespace ac::abi::guard {
namespace {
constexpr const char* kModule = "Abi.StructGuard";
std::atomic<std::uint32_t> g_rejections{0};
std::atomic<bool> g_contradicted{false};
std::atomic<std::size_t> g_checked{0};
}  // namespace

const char* ReasonText(Reason r) {
    switch (r) {
    case Reason::Ok: return "ok";
    case Reason::TableMismatch:
        return "the build's ABI table contradicts the layout this DLL was compiled with";
    case Reason::NullObject: return "null object";
    case Reason::BoolBlockNotBoolean: return "bool block holds non-boolean bytes (layout shifted?)";
    case Reason::ReleaseStateOutOfRange: return "releaseState out of range";
    case Reason::PackageCountAbsurd: return "existInPackageNums absurd";
    case Reason::VectorSizeExceedsAlloc: return "vector size exceeds allocationCount";
    case Reason::VectorMemoryNull: return "vector has elements but null storage";
    case Reason::VectorMemoryMisaligned: return "vector storage is misaligned";
    case Reason::VectorAllocAbsurd: return "vector allocationCount absurd";
    case Reason::DepotEntryImplausible:
        return "depot record does not look like a DepotEntry (layout shifted?)";
    }
    return "unknown";
}

bool LayoutContradicted() { return g_contradicted.load(std::memory_order_relaxed); }

std::size_t TableFieldsChecked() { return g_checked.load(std::memory_order_relaxed); }

std::size_t ApplyTable(const std::unordered_map<std::string, std::uint32_t>& published,
                       const std::string& source) {
    std::size_t matched = 0;
    std::size_t mismatched = 0;
    for (const auto& expected : kExpectedLayout) {
        const auto it = published.find(expected.key);
        if (it == published.end()) continue;  // older table: not an opinion
        if (it->second == expected.compiled) {
            ++matched;
            continue;
        }
        ++mismatched;
        AC_LOG_ERROR(kModule,
                     "ABI table (%s): %s is 0x%X in this build but this DLL was compiled "
                     "with 0x%zX. A struct cannot be re-laid-out at runtime, so every "
                     "guarded write is disabled for this session. AetherCore needs a "
                     "rebuild for this build of Steam.",
                     source.c_str(), expected.key, it->second, expected.compiled);
        diag::Record("abi_table_mismatch", std::string(expected.key) + " published=" +
                                               std::to_string(it->second) + " compiled=" +
                                               std::to_string(expected.compiled));
    }

    g_checked.store(matched + mismatched, std::memory_order_relaxed);
    if (mismatched > 0) {
        g_contradicted.store(true, std::memory_order_relaxed);
    } else if (matched > 0) {
        AC_LOG_INFO(kModule,
                    "ABI table (%s): %zu field offset(s) confirm the compiled layout.",
                    source.c_str(), matched);
    }
    return matched;
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

void ResetCounters() {
    g_rejections.store(0, std::memory_order_relaxed);
    g_contradicted.store(false, std::memory_order_relaxed);
    g_checked.store(0, std::memory_order_relaxed);
}

}  // namespace ac::abi::guard
