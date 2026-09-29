#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <string>

// ---------------------------------------------------------------------------
// CNetPacket ABI — runtime-resolved field layout.
//
// WHY THIS FILE EXISTS
// --------------------
// Steam client build 1790380355 (beta) inserted two uint32 "version stamps"
// after CNetPacket::m_hConnection, shifting every field below them by +8:
//
//     field          stable 1788652215      beta 1790380355
//     m_hConnection  +0x00                  +0x00
//     (new u32 x2)   --                     +0x04, +0x08
//     m_pubData      +0x08                  +0x10
//     m_cubData      +0x10                  +0x18
//     m_cRef         +0x14                  +0x1C
//     m_pubCopy      +0x18                  +0x20
//
// Proven by disassembly of CNetPacket::AddRef (stable 0xE64B90 / beta
// 0xE82730) and of CNetPacket::GetMsgNetPacket (stable 0xD11C50 / beta
// 0xD243F0), whose first two instructions literally read the two fields we
// care about:
//
//     mov r8d, [rcx + 0x18]    ; m_cubData (beta)
//     mov r12, [rcx + 0x10]    ; m_pubData (beta)
//
// The old code hard-coded the stable offsets and WROTE through them, so on
// beta it stored a pointer over a version stamp and dereferenced a small
// integer as a buffer -> access violation before Steam finished booting.
//
// DESIGN RULES (do not weaken them)
// ---------------------------------
//  1. CNetPacket is OPAQUE. No field is declared anywhere; no sizeof is taken.
//     The only way in is Data()/Size() below, which apply a runtime offset.
//  2. m_cubData always sits 8 bytes after m_pubData and m_cRef 4 bytes after
//     that, on every build seen so far, so ONE offset describes the layout.
//  3. The whole layout state lives in ONE atomic word. A separate "resolved"
//     flag next to the offset would let another Steam thread observe the flag
//     set while the offset was still zero and write a field at offset 0 —
//     straight over m_hConnection.
//  4. There is NO fallback to a compiled default. The default is correct on
//     one build and lethal on the other: that is precisely the bug this file
//     exists to remove. Unknown layout => the feature is disabled.
//  5. The validation predicate is Valve's own (see CandidateMatches): the same
//     checks GetMsgNetPacket performs one instruction later. We do not invent
//     heuristics when the client ships one.
//
// Everything above the Windows layer is pure and header-only so the unit
// tests can drive it with synthetic packets (see tests, case "netpacket").
// ---------------------------------------------------------------------------
namespace ac::steam {
struct CNetPacket;  // opaque by design — never define it
}

namespace ac::abi::netpkt {

// ---- layout table ---------------------------------------------------------
// One row per known layout. Valve will shift this again; a new row should be
// the whole change.
struct Layout {
    const char* name;
    std::uint32_t dataOff;
};

inline constexpr Layout kLayouts[] = {
    {"stable", 0x08},
    {"beta", 0x10},
};

//: Layouts learned at runtime from the build's own ABI table, on top of the
//: two we know. This is the point of phase 4: the table PROPOSES a layout we
//: have never compiled in, the probe DISPOSES by checking it against a live
//: packet. A build that shifts the struct again therefore needs a new table,
//: not a new AetherCore.
inline constexpr int kMaxExtraLayouts = 4;

inline constexpr std::uint32_t kUnresolved = 0u;
inline constexpr std::uint32_t kDisabled = 0xFFFFFFFFu;

// Derived offsets: the two companions of dataOff.
inline constexpr std::uint32_t SizeOffFor(std::uint32_t dataOff) { return dataOff + 8; }
inline constexpr std::uint32_t RefOffFor(std::uint32_t dataOff) { return dataOff + 0x0C; }

// ---- probe tunables (public so tests can reason about them) ---------------
inline constexpr std::uint32_t kMsgHdrBytes = 8;             // eMsg + headerLength
inline constexpr std::uint32_t kProtoFlag = 0x80000000u;     // high bit of eMsg
inline constexpr std::uint32_t kMaxFrameBytes = 64u * 1024u * 1024u;
inline constexpr std::uint32_t kMaxHeaderBytes = 8192;
inline constexpr std::uint32_t kMinHeaderBytes = 2;
inline constexpr std::int32_t kMinRefCount = 1;
inline constexpr std::int32_t kMaxRefCount = 1000000;
inline constexpr std::uintptr_t kMinPtr = 0x10000ull;
inline constexpr std::uintptr_t kMaxPtr = 0x7FFFFFFF0000ull;
inline constexpr int kMaxProbeAttempts = 512;
// Consecutive write-barrier rejections tolerated before the layout is
// declared wrong and the wire features shut down. One rejection is a weird
// packet; eight in a row means the object is not what the latched layout
// says it is, and continuing would corrupt Steam.
inline constexpr int kMaxWriteMismatches = 8;

// Readability oracle. The Windows implementation is DefaultReadable (in the
// .cpp); tests pass their own so the state machine stays portable.
using ReadableFn = bool (*)(const void* addr, std::size_t bytes, void* ctx);

// True when `dataOff` describes THIS packet. Reads nothing it has not first
// proved readable. This is Valve's predicate from GetMsgNetPacket, plus the
// object-level sanity checks (pointer range, refcount) that the client can
// skip because it owns the object and we cannot.
inline bool CandidateMatches(const void* packet, std::uint32_t dataOff,
                             ReadableFn readable, void* ctx) {
    if (!packet || !readable) return false;
    const auto* base = static_cast<const std::uint8_t*>(packet);

    // data (8) + size (4) + refcount (4)
    if (!readable(base + dataOff, 0x10, ctx)) return false;

    const auto* data = *reinterpret_cast<const std::uint8_t* const*>(base + dataOff);
    const std::uint32_t len =
        *reinterpret_cast<const std::uint32_t*>(base + SizeOffFor(dataOff));
    const std::int32_t ref =
        *reinterpret_cast<const std::int32_t*>(base + RefOffFor(dataOff));

    const auto addr = reinterpret_cast<std::uintptr_t>(data);
    if (addr < kMinPtr || addr >= kMaxPtr) return false;
    if (len < kMsgHdrBytes || len > kMaxFrameBytes) return false;
    if (ref < kMinRefCount || ref > kMaxRefCount) return false;

    if (!readable(data, kMsgHdrBytes, ctx)) return false;

    // Valve: cmp r8d,8 / cmp dword[data],0 / jge -> not proto
    //        mov edx,[data+4] / cmp rdx, len-8 / jg -> malformed
    const std::uint32_t rawEMsg = *reinterpret_cast<const std::uint32_t*>(data);
    const std::uint32_t headerLen = *reinterpret_cast<const std::uint32_t*>(data + 4);
    if ((rawEMsg & kProtoFlag) == 0) return false;  // non-proto frame: no evidence
    const std::uint32_t eMsg = rawEMsg & ~kProtoFlag;
    if (eMsg == 0 || eMsg >= 0x10000) return false;
    if (headerLen < kMinHeaderBytes || headerLen > kMaxHeaderBytes) return false;
    if (headerLen > len - kMsgHdrBytes) return false;

    return readable(data, kMsgHdrBytes + headerLen, ctx);
}

// Structural check used by the WRITE barrier, as opposed to CandidateMatches
// which identifies the layout.
//
// The difference matters and cost us six skipped rewrites in the first
// production session: identification demands a protobuf frame, because a
// protobuf header is the only thing that lets us tell two candidate layouts
// apart with certainty. But Steam also carries legacy struct packets (see
// CStructNetPacket / netpacket_legacy_struct.h), and those are perfectly
// legitimate objects that simply have no protobuf flag. Refusing to write
// them was over-caution: the layout is already known at that point, so the
// gate only has to answer "is this still a CNetPacket?", not "which layout
// is it?".
inline bool LayoutPlausible(const void* packet, std::uint32_t dataOff, ReadableFn readable,
                            void* ctx) {
    if (!packet || !readable) return false;
    const auto* base = static_cast<const std::uint8_t*>(packet);
    if (!readable(base + dataOff, 0x10, ctx)) return false;

    const auto* data = *reinterpret_cast<const std::uint8_t* const*>(base + dataOff);
    const std::uint32_t len =
        *reinterpret_cast<const std::uint32_t*>(base + SizeOffFor(dataOff));
    const std::int32_t ref =
        *reinterpret_cast<const std::int32_t*>(base + RefOffFor(dataOff));

    const auto addr = reinterpret_cast<std::uintptr_t>(data);
    if (addr < kMinPtr || addr >= kMaxPtr) return false;
    if (len < kMsgHdrBytes || len > kMaxFrameBytes) return false;
    if (ref < kMinRefCount || ref > kMaxRefCount) return false;
    return readable(data, kMsgHdrBytes, ctx);
}

// ---- resolver state machine (pure, testable) ------------------------------
//
// Latches only when exactly one candidate matches AND the same candidate also
// won the previous packet. Ambiguity is the one thing we must never latch on;
// requiring two agreeing packets costs at most one early proto message.
class Resolver {
public:
    enum class Step {
        NoEvidence,       // non-proto frame or nothing matched: carries no information
        Ambiguous,        // several candidates matched: standing agreement discarded
        AwaitingConfirm,  // one candidate matched, needs a second agreeing packet
        Latched,          // layout decided
        Disabled,         // gave up: no field will ever be touched again
    };

    std::uint32_t State() const { return state_.load(std::memory_order_relaxed); }

    // Adds a layout the compiled table does not know about (from the per-build
    // ABI section). Ignored once resolved, and never trusted without the probe.
    bool AddCandidate(std::uint32_t dataOff) {
        if (dataOff == kUnresolved || dataOff == kDisabled) return false;
        for (const auto& known : kLayouts) {
            if (known.dataOff == dataOff) return true;  // already a candidate
        }
        const int count = extraCount_.load(std::memory_order_relaxed);
        for (int i = 0; i < count; ++i) {
            if (extra_[i].load(std::memory_order_relaxed) == dataOff) return true;
        }
        if (count >= kMaxExtraLayouts) return false;
        extra_[count].store(dataOff, std::memory_order_relaxed);
        extraCount_.store(count + 1, std::memory_order_relaxed);
        return true;
    }
    // How many live packets actually agreed before the latch: 1 means the
    // build hint supplied the standing agreement and a single packet confirmed
    // it, 2 means two consecutive packets agreed on their own.
    int Confirmations() const { return confirmations_.load(std::memory_order_relaxed); }
    bool IsResolved() const {
        const std::uint32_t v = State();
        return v != kUnresolved && v != kDisabled;
    }
    bool IsDisabled() const { return State() == kDisabled; }
    std::uint32_t DataOffset() const { return IsResolved() ? State() : 0u; }
    int Attempts() const { return attempts_.load(std::memory_order_relaxed); }

    const char* LayoutName() const {
        const std::uint32_t v = State();
        if (v == kUnresolved) return "unresolved";
        if (v == kDisabled) return "disabled";
        for (const auto& l : kLayouts) {
            if (l.dataOff == v) return l.name;
        }
        return "from-abi-table";
    }

    void Latch(std::uint32_t dataOff) { state_.store(dataOff, std::memory_order_relaxed); }

    // Terminal state. Deliberately NOT "fall back to the compiled default".
    void Disable() { state_.store(kDisabled, std::memory_order_relaxed); }

    void Reset() {
        extraCount_.store(0, std::memory_order_relaxed);
        state_.store(kUnresolved, std::memory_order_relaxed);
        agreed_.store(kUnresolved, std::memory_order_relaxed);
        attempts_.store(0, std::memory_order_relaxed);
        confirmations_.store(0, std::memory_order_relaxed);
        mismatches_.store(0, std::memory_order_relaxed);
        rejects_.store(0, std::memory_order_relaxed);
    }

    int WriteRejects() const { return rejects_.load(std::memory_order_relaxed); }

    // Write barrier. Called immediately before Aether mutates a packet: the
    // latched layout is re-checked against THIS object, so a packet that does
    // not look like the layout we latched is left alone instead of being
    // written through. Consecutive failures are terminal (see
    // kMaxWriteMismatches): a layout that stops matching is a layout that was
    // wrong, and the safe move is to stop touching packets, not to keep
    // trying. Returns true when the write may proceed.
    bool BeginWrite(const void* packet, ReadableFn readable, void* ctx) {
        if (!IsResolved() || !packet) return false;
        if (LayoutPlausible(packet, State(), readable, ctx)) {
            mismatches_.store(0, std::memory_order_relaxed);
            return true;
        }
        rejects_.fetch_add(1, std::memory_order_relaxed);
        if (mismatches_.fetch_add(1, std::memory_order_relaxed) + 1 >= kMaxWriteMismatches) {
            Disable();
        }
        return false;
    }

    // Seeds the candidate that a per-build table (or a known-SHA hint) claims
    // is correct. It is an OPINION, not a decision: the probe must still match
    // it on a live packet before anything is written. The only effect is that
    // the confirmation arrives one packet earlier.
    void Hint(std::uint32_t dataOff) {
        if (State() != kUnresolved) return;
        agreed_.store(dataOff, std::memory_order_relaxed);
        // The hint stands in for the first agreeing packet, so the next live
        // packet that matches it completes the confirmation.
        confirmations_.store(0, std::memory_order_relaxed);
    }

    std::uint32_t Hinted() const { return agreed_.load(std::memory_order_relaxed); }

    // Feeds one live packet to the resolver.
    Step Observe(const void* packet, ReadableFn readable, void* ctx) {
        if (IsDisabled()) return Step::Disabled;
        if (IsResolved()) return Step::Latched;

        if (attempts_.fetch_add(1, std::memory_order_relaxed) + 1 > kMaxProbeAttempts) {
            Disable();
            return Step::Disabled;
        }

        std::uint32_t winner = kUnresolved;
        int passes = 0;
        for (const auto& layout : kLayouts) {
            if (CandidateMatches(packet, layout.dataOff, readable, ctx)) {
                ++passes;
                winner = layout.dataOff;
            }
        }
        const int extras = extraCount_.load(std::memory_order_relaxed);
        for (int i = 0; i < extras; ++i) {
            const std::uint32_t dataOff = extra_[i].load(std::memory_order_relaxed);
            if (CandidateMatches(packet, dataOff, readable, ctx)) {
                ++passes;
                winner = dataOff;
            }
        }

        if (passes == 0) {
            // What a non-protobuf frame looks like. It is not evidence either
            // way, so leave any standing agreement intact: discarding it here
            // would let one interleaved non-proto packet restart the
            // confirmation, which is exactly what early login traffic does.
            return Step::NoEvidence;
        }
        if (passes > 1) {
            // Genuine ambiguity — and that IS evidence: do not trust the
            // standing agreement.
            agreed_.store(kUnresolved, std::memory_order_relaxed);
            confirmations_.store(0, std::memory_order_relaxed);
            return Step::Ambiguous;
        }
        if (agreed_.load(std::memory_order_relaxed) != winner) {
            agreed_.store(winner, std::memory_order_relaxed);
            confirmations_.store(1, std::memory_order_relaxed);
            return Step::AwaitingConfirm;
        }

        confirmations_.fetch_add(1, std::memory_order_relaxed);
        Latch(winner);
        return Step::Latched;
    }

private:
    // constinit-friendly: a hook that fires before dynamic initialization must
    // not read a garbage word.
    std::atomic<std::uint32_t> state_{kUnresolved};
    std::atomic<std::uint32_t> agreed_{kUnresolved};
    std::atomic<int> attempts_{0};
    std::atomic<int> confirmations_{0};
    std::atomic<int> mismatches_{0};
    std::atomic<int> rejects_{0};
    std::atomic<int> extraCount_{0};
    std::atomic<std::uint32_t> extra_[kMaxExtraLayouts]{};
};

// The process-wide resolver.
//
// Inline with a function-local static: one instance across the program, and
// no symbol for a test binary to link. The hooks can fire on Steam threads
// that exist before our dynamic initialisation, which is why it is a
// function-local static and not a namespace-scope object.
inline Resolver& Global() {
    static Resolver instance;
    return instance;
}

// ---- field accessors ------------------------------------------------------
//
// References, so reads, writes, save/restore pairs and in-place repoints are
// a plain substitution at the call sites.
namespace detail {
// Returned while the layout is unknown. Every real call site sits behind the
// EnsureResolved() gate, so this is unreachable today; it exists so that a
// call accidentally added above the gate corrupts a dead global instead of a
// live Steam object.
inline std::uint8_t* g_trashData = nullptr;
inline std::uint32_t g_trashSize = 0;
}  // namespace detail

inline std::uint8_t*& Data(steam::CNetPacket* p) {
    const std::uint32_t off = Global().State();
    if (!p || off == kUnresolved || off == kDisabled) return detail::g_trashData;
    return *reinterpret_cast<std::uint8_t**>(reinterpret_cast<std::uint8_t*>(p) + off);
}

inline std::uint32_t& Size(steam::CNetPacket* p) {
    const std::uint32_t off = Global().State();
    if (!p || off == kUnresolved || off == kDisabled) return detail::g_trashSize;
    return *reinterpret_cast<std::uint32_t*>(reinterpret_cast<std::uint8_t*>(p) +
                                             SizeOffFor(off));
}

// ---- Windows glue (implemented in NetPacketAbi.cpp) -----------------------

// VirtualQuery-backed readability oracle.
bool DefaultReadable(const void* addr, std::size_t bytes, void* ctx);

// Feeds `packet` to the global resolver and returns true when the layout is
// known, i.e. when Data()/Size() may be touched. Call it at the TOP of every
// hook that handles a CNetPacket, and pass the packet through untouched when
// it returns false. Logs each state transition exactly once.
bool EnsureResolved(const steam::CNetPacket* packet);

// Seeds the resolver from the per-build ABI table published alongside the
// patterns (phase 4). Offsets are checked for internal consistency
// (m_cubData == m_pubData + 8, m_cRef == m_pubData + 0xC) before being
// accepted as a candidate: a table that disagrees with the shape the
// accessors assume is a table we must not act on. Returns true when the
// layout was accepted as a probe candidate.
bool SeedFromAbiTable(std::uint32_t dataOff, std::uint32_t cubOff, std::uint32_t refOff,
                      const std::string& source);

// Where the latched layout ultimately came from: "abi-table", "build-hint" or
// "probe". Diagnostic only — the probe always has the last word.
const char* HintSource();

// Seeds the resolver from the running build's steamclient SHA-256 when it is
// one we have disassembled. Pure optimisation: the probe still has to agree.
// Called once during init, before any hook is installed.
void SeedFromBuild(const std::string& steamclientSha256);

// Human-readable state for status.json / logs: "stable", "beta", "custom",
// "unresolved", "disabled".
const char* LayoutName();
std::uint32_t ResolvedDataOffset();
// 1 = build hint + one confirming packet, 2 = two consecutive packets.
int ProbeConfirmations();
int WriteRejects();

// Write barrier for the hooks: true when `packet` still matches the latched
// layout and may be mutated. Logs the shutdown if the barrier gives up.
bool BeginWrite(const steam::CNetPacket* packet);
bool IsResolved();
bool IsDisabled();
int ProbeAttempts();

}  // namespace ac::abi::netpkt
