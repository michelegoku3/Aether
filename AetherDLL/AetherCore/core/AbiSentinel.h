#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <string>

// ---------------------------------------------------------------------------
// ABI sentinel — "is this address really the start of a function?"
//
// WHY THIS FILE EXISTS
// --------------------
// A pattern table can be wrong in a way the signature check cannot catch:
// the published RVA may point INTO a function instead of at its entry. It has
// already happened in this project — an override pinned `RecvPkt` to
// 0x5BC460, which on beta 1790380355 is not a function start at all (it sits
// inside the fragment 0x5BC32A-0x5BD139). MinHook happily writes a jump
// there, overwriting live instructions in the middle of a basic block; Steam
// then crashes somewhere unrelated, hours later, with no usable evidence.
//
// The sentinel runs BEFORE any hook is installed and answers one question
// with two independent sources of truth:
//
//   1. `.pdata` (the exception directory). Every non-leaf x64 function has a
//      RUNTIME_FUNCTION entry: if the RVA equals a BeginAddress, it is a
//      function start, full stop. If the RVA falls strictly INSIDE an entry,
//      it is provably NOT a start -> reject.
//   2. The preceding byte, for leaf functions that have no unwind data (e.g.
//      GetPipeClient, beta 0x89F270, which `.pdata` does not list). A real
//      function is preceded by the end of the previous one: MSVC pads with
//      0xCC (int3), or the previous function ends in 0xC3 (ret), or the gap
//      is zero-filled. Mid-function addresses are preceded by ordinary code.
//      Measured on the shipped binaries: every genuine entry point in our
//      catalogue is preceded by 0xCC (or 0xC3), and the known-bad 0x5BC460 is
//      preceded by `48 8B D3`.
//
// Verdicts are advisory data, not exceptions: a rejected address means "do
// not hook this", never "abort". Aether keeps running with one feature less.
//
// The classifier is pure and header-visible so the unit tests can drive it
// with real byte sequences (see tests, case "sentinel"). Only Verify() needs
// Windows.
// ---------------------------------------------------------------------------
namespace ac::abi::sentinel {

// One `.pdata` RUNTIME_FUNCTION, reduced to what we need. The array in a PE
// is sorted by BeginAddress, which is what makes the lookup a binary search.
struct PdataEntry {
    std::uint32_t begin = 0;
    std::uint32_t end = 0;
};

enum class Verdict {
    Ok,                     // exact .pdata BeginAddress — proven entry point
    OkLeaf,                 // no unwind data, but preceded by a function boundary
    NotFunctionStart,       // strictly inside a .pdata range — proven wrong
    NoPredecessorBoundary,  // no unwind data and preceded by live code
    PaddingAtTarget,        // the target itself is padding (0xCC / zero fill)
    OutsideCode,            // not inside the module's executable section
    NoBytes,                // could not read the bytes to judge
};

//: Inline for the same reason as guard::ReasonText: a pure switch should not
//: force a test binary to link the Windows side.
inline const char* VerdictText(Verdict v) {
    switch (v) {
    case Verdict::Ok: return "function start (.pdata)";
    case Verdict::OkLeaf: return "function start (leaf, preceded by a boundary)";
    case Verdict::NotFunctionStart: return "address is INSIDE another function, not its start";
    case Verdict::NoPredecessorBoundary: return "no unwind entry and preceded by live code";
    case Verdict::PaddingAtTarget: return "address points at padding";
    case Verdict::OutsideCode: return "address is outside the executable section";
    case Verdict::NoBytes: return "target bytes are unreadable";
    }
    return "unknown";
}

inline bool Accepted(Verdict v) { return v == Verdict::Ok || v == Verdict::OkLeaf; }

// Pure classifier.
//   rva       address under test, relative to the module base
//   inCode    caller already checked it lands in an executable section
//   sorted    `.pdata` entries, sorted by `begin` (may be null/empty)
//   prevByte  the byte at rva-1
//   first     the first bytes at rva (>= 1), used to spot padding
inline Verdict Classify(std::uint32_t rva, bool inCode, const PdataEntry* sorted,
                        std::size_t count, std::uint8_t prevByte, const std::uint8_t* first,
                        std::size_t firstLen) {
    if (!inCode) return Verdict::OutsideCode;
    if (!first || firstLen == 0) return Verdict::NoBytes;

    // Padding is never a function: an RVA that lands here is stale by a few
    // bytes, which is the signature of a table generated against another build.
    if (first[0] == 0xCC) return Verdict::PaddingAtTarget;
    if (firstLen >= 4 && first[0] == 0 && first[1] == 0 && first[2] == 0 && first[3] == 0) {
        return Verdict::PaddingAtTarget;
    }

    if (sorted && count > 0) {
        // Last entry whose begin <= rva.
        std::size_t lo = 0, hi = count;
        while (lo < hi) {
            const std::size_t mid = lo + (hi - lo) / 2;
            if (sorted[mid].begin <= rva) lo = mid + 1; else hi = mid;
        }
        if (lo > 0) {
            const PdataEntry& e = sorted[lo - 1];
            if (e.begin == rva) return Verdict::Ok;
            if (rva < e.end) return Verdict::NotFunctionStart;
        }
        // Falls in a gap between entries: a leaf function, or the tail of a
        // fragmented function. The predecessor test decides.
    }

    if (prevByte == 0xCC || prevByte == 0xC3 || prevByte == 0x00) return Verdict::OkLeaf;
    return Verdict::NoPredecessorBoundary;
}

// ---- Windows glue (AbiSentinel.cpp) --------------------------------------

// Verifies a resolved hook target inside `module`. Logs the outcome, updates
// the counters, and fills `detail` with a short human-readable reason when the
// verdict is a rejection. Returns the verdict; callers should treat
// !Accepted(v) as "skip this hook".
Verdict Verify(const std::string& name, const void* target, void* module, std::string* detail);

// Counters for status.json.
std::uint32_t VerifiedCount();
std::uint32_t RejectedCount();

// Test seam: lets the unit tests reset the counters.
void ResetCounters();

}  // namespace ac::abi::sentinel
