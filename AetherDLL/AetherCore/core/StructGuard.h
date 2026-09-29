#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <string>
#include <unordered_map>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Struct guards — "does this object actually look like the struct we think?"
//
// Phase 3 of the ABI work. NetPacketAbi solved the packet path; these are the
// other two places where Aether writes into Valve's memory:
//
//   * PackageInfo (package 0 injection) — writes into a CUtlVector. Getting
//     size/allocationCount wrong means writing past the allocation: heap
//     corruption, and a crash minutes later with no usable stack.
//   * AppOwnership (CheckAppOwnership out-param) — the dangerous one does NOT
//     crash. A shifted layout just makes Steam believe wrong answers about
//     ownership, silently.
//
// The layouts below are proven by disassembly of both shipped builds
// (stable 1788652215 / beta 1790380355) and, unlike CNetPacket, they did NOT
// move between them:
//
//   CheckAppOwnership (stable 0x9CE9E0 / beta 0x9DC160), identical stores:
//       +0x00..0x14  six u32
//       +0x18 word + 0x1A byte   purchaseCountryCode
//       +0x1C timeStamp, +0x20 timeExpire
//       +0x24..0x35  EIGHTEEN bool bytes, contiguous, size_bytes = 0x36
//       (note: `mov word ptr [rbx+0x33], 0x101` writes two of them at once —
//        a scan for `mov [rbx+d], al` alone finds only 12 of the 18)
//
//   LoadPackage (stable 0x4B1A10 / beta 0x4C49E0):
//       appIdVec   base +0x40  -> memory +0x40, allocationCount +0x48, size +0x50
//       depotIdVec base +0x58  -> memory +0x58, allocationCount +0x60, size +0x68
//       elements u32, stride 4; growth via CUtlMemoryGrow(vec, delta)
//
// So these guards are not (yet) about a layout that changed: they are the
// tripwire for the day it does. They run on a live object and answer with a
// reason, so the caller can refuse to write instead of corrupting Steam.
//
// Pure and header-only: the unit tests drive them with synthetic objects.
// ---------------------------------------------------------------------------
namespace ac::abi::guard {

// ---- compile-time fence ---------------------------------------------------
// If someone reorders a field, the build breaks here rather than in a user's
// Steam. The numbers are the disassembly, not our wishes.
static_assert(offsetof(steam::AppOwnership, packageId) == 0x00, "AppOwnership layout");
static_assert(offsetof(steam::AppOwnership, timeStamp) == 0x1C, "AppOwnership layout");
static_assert(offsetof(steam::AppOwnership, timeExpire) == 0x20, "AppOwnership layout");
static_assert(offsetof(steam::AppOwnership, ownsLicense) == 0x24, "AppOwnership bool block");
static_assert(offsetof(steam::AppOwnership, freeLicense) == 0x28, "AppOwnership bool block");
// +0x2F, not +0x31: the byte at +0x31 has unknown meaning and reading it as
// "borrowed" made Steam show the family-sharing banner on .lua apps.
static_assert(offsetof(steam::AppOwnership, borrowed) == 0x2F, "AppOwnership borrowed");
static_assert(offsetof(steam::AppOwnership, familyShared) == 0x35, "AppOwnership bool block");
static_assert(sizeof(steam::AppOwnership) == 0x38, "AppOwnership size (0x36 used + padding)");

static_assert(offsetof(steam::PackageInfo, status) == 0x18, "PackageInfo layout");
static_assert(offsetof(steam::PackageInfo, appIdVec) == 0x40, "PackageInfo layout");
static_assert(offsetof(steam::PackageInfo, depotIdVec) == 0x58, "PackageInfo layout");
static_assert(sizeof(steam::CUtlVector<steam::AppId>) == 0x18, "CUtlVector stride");
static_assert(sizeof(steam::DepotEntry) == 0x20, "DepotEntry stride");
static_assert(offsetof(steam::DepotEntry, manifestGid) == 0x08, "DepotEntry layout");
static_assert(offsetof(steam::DepotEntry, manifestSize) == 0x10, "DepotEntry layout");
static_assert(offsetof(steam::CUtlVector<steam::AppId>, size) == 0x10, "CUtlVector::size");

// First and last byte of the contiguous 18-bool block.
inline constexpr std::size_t kOwnershipBoolFirst = 0x24;
inline constexpr std::size_t kOwnershipBoolCount = 18;

// ---- the published table --------------------------------------------------
//
// Phase 4, second half. The generator now extracts these offsets from the
// build's own code, so we can finally CHECK the layout this DLL was compiled
// against instead of merely asserting it at build time.
//
// What we deliberately do NOT do: adapt. A C++ struct cannot be re-laid-out at
// runtime, so if the published table disagrees with the compiled layout the
// only safe move is to stop writing and say so loudly. That converts a silent
// corruption into a visible, diagnosable degradation — which is the whole
// point of the exercise.
struct ExpectedLayout {
    const char* key;       // "Struct.field" as published
    std::size_t compiled;  // what this binary was built with
};

//: Checked against the table at startup. Every entry is proven by
//: disassembly of both shipped builds (round 3/5).
inline constexpr ExpectedLayout kExpectedLayout[] = {
    {"AppOwnership.bool_block_start", 0x24},
    {"AppOwnership.size_bytes", 0x36},
    {"AppOwnership.timeStamp", 0x1C},
    {"AppOwnership.timeExpire", 0x20},
    {"AppOwnership.borrowed", 0x2F},
    {"DepotEntry.manifestGid", 0x08},
    {"DepotEntry.manifestSize", 0x10},
    {"PackageInfo.status", 0x18},
    {"PackageInfo.appIdVec", 0x40},
    {"PackageInfo.depotIdVec", 0x58},
    {"CUtlVector.allocationCount", 0x08},
    {"CUtlVector.count", 0x10},
};

// ---- results --------------------------------------------------------------
enum class Reason {
    Ok,
    TableMismatch,  // the build's ABI table contradicts the compiled layout
    NullObject,
    BoolBlockNotBoolean,   // a "bool" byte holds something other than 0/1
    ReleaseStateOutOfRange,
    PackageCountAbsurd,
    VectorSizeExceedsAlloc,
    VectorMemoryNull,
    VectorMemoryMisaligned,
    VectorAllocAbsurd,
    DepotEntryImplausible,
};

//: Text for a reason. Inline: it is a pure switch, and keeping it in the
//: header means a test binary does not have to link the Windows glue just to
//: print a message.
inline const char* ReasonText(Reason r) {
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
inline bool Passed(Reason r) { return r == Reason::Ok; }

// ---- tunables -------------------------------------------------------------
// Package 0 on a heavy account holds ~1200 entries; a million is absurd by
// three orders of magnitude, which is what we want from a tripwire.
inline constexpr std::uint32_t kMaxVectorElements = 1u << 20;
inline constexpr std::uint32_t kMaxExistInPackageNums = 4096;
inline constexpr std::uint32_t kMaxReleaseState = 5;
//: A depot larger than this is not a depot. Steam's biggest shipping depots
//: are a few hundred GB; 4 TB is three orders of magnitude of headroom and
//: still catches a pointer read as a size.
inline constexpr std::uint64_t kMaxDepotBytes = 4ull * 1024 * 1024 * 1024 * 1024;

// ---- AppOwnership ---------------------------------------------------------
//
// The decisive check is the bool block: eighteen consecutive bytes that Valve
// only ever sets to 0 or 1. If the struct shifted, or the pointer is not an
// AppOwnership at all, those bytes hold pointer fragments or counters and the
// test fails immediately. It costs 18 byte comparisons.
// ---- state and reporting --------------------------------------------------
//
// The POLICY (what counts as a mismatch, what to refuse, what to count) lives
// here, inline, so the unit tests exercise the real thing. Only the
// REPORTING — writing a log line, recording a diagnostic — is left to the
// platform layer, because a test binary has no logger and should not need
// one. Three hooks, implemented in StructGuard.cpp and stubbed in tests.
void ReportRejection(const char* what, Reason reason);
void ReportTableMismatch(const char* field, std::uint32_t published, std::size_t compiled,
                         const std::string& source);
void ReportTableAgrees(std::size_t matched, const std::string& source);

namespace detail {
inline std::atomic<bool> g_contradicted{false};
inline std::atomic<std::uint32_t> g_rejections{0};
inline std::atomic<std::size_t> g_tableFields{0};
}  // namespace detail

// Set when the published table disagrees with the compiled layout: every
// guarded write is refused from that point on.
inline bool LayoutContradicted() {
    return detail::g_contradicted.load(std::memory_order_relaxed);
}

inline Reason CheckOwnership(const steam::AppOwnership* o) {
    if (!o) return Reason::NullObject;
    if (LayoutContradicted()) return Reason::TableMismatch;

    const auto* bytes = reinterpret_cast<const std::uint8_t*>(o);
    for (std::size_t i = 0; i < kOwnershipBoolCount; ++i) {
        if (bytes[kOwnershipBoolFirst + i] > 1) return Reason::BoolBlockNotBoolean;
    }
    if (static_cast<std::uint32_t>(o->releaseState) > kMaxReleaseState) {
        return Reason::ReleaseStateOutOfRange;
    }
    if (o->existInPackageNums > kMaxExistInPackageNums) return Reason::PackageCountAbsurd;
    return Reason::Ok;
}

// ---- CUtlVector / PackageInfo --------------------------------------------
template <typename T>
inline Reason CheckVector(const steam::CUtlVector<T>& v) {
    if (v.mem.allocationCount > kMaxVectorElements) return Reason::VectorAllocAbsurd;
    if (v.size > v.mem.allocationCount) return Reason::VectorSizeExceedsAlloc;
    if (v.size > 0 && v.mem.memory == nullptr) return Reason::VectorMemoryNull;
    if (v.mem.memory != nullptr &&
        (reinterpret_cast<std::uintptr_t>(v.mem.memory) & (alignof(T) - 1)) != 0) {
        return Reason::VectorMemoryMisaligned;
    }
    return Reason::Ok;
}

// ---- DepotEntry -----------------------------------------------------------
//
// Why this one needs a guard at all: `manifestGid` is the id of the exact
// content snapshot Steam is about to download, and Aether REPLACES it. Write
// it at the wrong offset and the consequences are not a crash — they are a
// download of the wrong build, or a corrupted dependency table that Steam
// then acts on. The record is 32 bytes with a very distinctive shape
// (two ids, two 64-bit quantities, three booleans), so a shifted layout is
// easy to spot before touching anything.
inline Reason CheckDepotEntry(const steam::DepotEntry& e) {
    if (LayoutContradicted()) return Reason::TableMismatch;
    // A real row always identifies a depot; a zeroed tail row is not a defect.
    if (e.depotId == 0 && e.manifestGid == 0 && e.manifestSize == 0) return Reason::Ok;
    if (e.depotId == 0) return Reason::DepotEntryImplausible;
    if (e.manifestSize > kMaxDepotBytes) return Reason::DepotEntryImplausible;
    // The three flags are booleans in the binary: anything else means the
    // record does not start where we think it does.
    if (e.lcsRequired > 1 || e.notNewTarget > 1 || e.sharedInstall > 1) {
        return Reason::DepotEntryImplausible;
    }
    return Reason::Ok;
}

// Checks the whole table before Aether rewrites any manifest id. Returns the
// first problem found, so one bad row stops the pass instead of being skipped
// silently.
inline Reason CheckDepotVector(const steam::CUtlVector<steam::DepotEntry>* vec) {
    if (!vec) return Reason::NullObject;
    const Reason shape = CheckVector(*vec);
    if (!Passed(shape)) return shape;
    if (!vec->mem.memory) return Reason::Ok;  // empty table: nothing to check
    for (std::uint32_t i = 0; i < vec->size; ++i) {
        const Reason row = CheckDepotEntry(vec->mem.memory[i]);
        if (!Passed(row)) return row;
    }
    return Reason::Ok;
}

inline Reason CheckPackage(const steam::PackageInfo* p) {
    if (!p) return Reason::NullObject;
    if (LayoutContradicted()) return Reason::TableMismatch;
    const Reason apps = CheckVector(p->appIdVec);
    if (!Passed(apps)) return apps;
    return CheckVector(p->depotIdVec);
}

// ---- counters (StructGuard.cpp) ------------------------------------------
// Every refusal is counted and surfaced in status.json: a guard that fires
// silently is a guard nobody acts on.
inline void CountRejection(const char* what, Reason r) {
    detail::g_rejections.fetch_add(1, std::memory_order_relaxed);
    ReportRejection(what, r);
}

// Compares the per-build ABI table with kExpectedLayout. Returns the number of
// fields that matched; any mismatch latches LayoutContradicted(). Fields
// absent from the table are neither a match nor a mismatch: older tables
// simply do not carry them.
inline std::size_t ApplyTable(const std::unordered_map<std::string, std::uint32_t>& published,
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
        ReportTableMismatch(expected.key, it->second, expected.compiled, source);
    }

    detail::g_tableFields.store(matched + mismatched, std::memory_order_relaxed);
    if (mismatched > 0) {
        detail::g_contradicted.store(true, std::memory_order_relaxed);
    } else if (matched > 0) {
        ReportTableAgrees(matched, source);
    }
    return matched;
}

inline std::size_t TableFieldsChecked() {
    return detail::g_tableFields.load(std::memory_order_relaxed);
}
inline std::uint32_t RejectionCount() {
    return detail::g_rejections.load(std::memory_order_relaxed);
}

inline void ResetCounters() {
    detail::g_rejections.store(0, std::memory_order_relaxed);
    detail::g_contradicted.store(false, std::memory_order_relaxed);
    detail::g_tableFields.store(0, std::memory_order_relaxed);
}

}  // namespace ac::abi::guard
