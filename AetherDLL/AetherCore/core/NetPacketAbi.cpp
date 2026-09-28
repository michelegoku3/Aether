#include "pch.h"
#include "core/NetPacketAbi.h"

#include <windows.h>

#include "core/Logger.h"
#include "utils/Strings.h"

namespace ac::abi::netpkt {
namespace {

constexpr const char* kModule = "Abi.NetPacket";

// Builds we have disassembled field by field. This table is a HINT ONLY: it
// never reaches Data()/Size() without the live probe agreeing first, so a
// wrong or stale row costs one packet of latency, never a wild write.
//
//   caba4826... = steamclient64.dll build 1788652215 (stable, Sep 3-6 2026)
//                 CNetPacket::AddRef 0xE64B90  -> m_cRef @ +0x14
//                 CNetPacket::GetMsgNetPacket 0xD11C50
//   44715171... = steamclient64.dll build 1790380355 (beta, Sep 25 2026)
//                 CNetPacket::AddRef 0xE82730  -> m_cRef @ +0x1C
//                 GetMsgNetPacket 0xD243F0: mov r8d,[rcx+0x18] / mov r12,[rcx+0x10]
struct BuildHint {
    const char* sha256;
    std::uint32_t dataOff;
    const char* build;
};

constexpr BuildHint kBuildHints[] = {
    {"caba4826aa3501039d095aee1843a6bfb270fb43a3ab4455b2d6733223579fee", 0x08, "1788652215"},
    {"4471517105b1fbd603441b38db04570d185a9c993e3aa54b5cc237c3eb1fe51f", 0x10, "1790380355"},
};

std::atomic<bool> g_latchLogged{false};
std::atomic<bool> g_disableLogged{false};
//: "abi-table" | "build-hint" | "probe" — how the candidate that won was
//: proposed. Never how it was *decided*: that is always the probe.
std::atomic<const char*> g_hintSource{"probe"};

}  // namespace

Resolver& Global() {
    // Function-local static: the hooks can fire on Steam threads that exist
    // before our dynamic initialization runs.
    static Resolver instance;
    return instance;
}

bool DefaultReadable(const void* addr, std::size_t bytes, void* /*ctx*/) {
    if (!addr || bytes == 0) return false;

    const auto start = reinterpret_cast<std::uintptr_t>(addr);
    if (start < kMinPtr || start >= kMaxPtr) return false;
    if (kMaxPtr - start < bytes) return false;  // wrap

    const std::uintptr_t end = start + bytes;
    std::uintptr_t cursor = start;

    // A range can straddle several regions; every one of them must be
    // committed and readable.
    while (cursor < end) {
        MEMORY_BASIC_INFORMATION mbi{};
        if (::VirtualQuery(reinterpret_cast<LPCVOID>(cursor), &mbi, sizeof(mbi)) != sizeof(mbi)) {
            return false;
        }
        if (mbi.State != MEM_COMMIT) return false;
        constexpr DWORD kNoRead = PAGE_NOACCESS | PAGE_GUARD;
        if ((mbi.Protect & kNoRead) != 0 || mbi.Protect == 0) return false;

        const auto regionEnd =
            reinterpret_cast<std::uintptr_t>(mbi.BaseAddress) + mbi.RegionSize;
        if (regionEnd <= cursor) return false;  // defensive: no progress
        cursor = regionEnd;
    }
    return true;
}

bool EnsureResolved(const steam::CNetPacket* packet) {
    Resolver& r = Global();
    if (r.IsResolved()) return true;
    if (r.IsDisabled()) return false;
    if (!packet) return false;

    const Resolver::Step step = r.Observe(packet, &DefaultReadable, nullptr);

    switch (step) {
    case Resolver::Step::Latched:
        if (!g_latchLogged.exchange(true)) {
            // The wording must match what actually happened: with a build hint
            // one confirming packet is enough, without one it takes two
            // agreeing packets. Reporting "two consecutive packets" after a
            // single attempt reads like a contradiction in the log and hides
            // whether the hint was in play.
            const char* how = r.Confirmations() >= 2
                                  ? "confirmed by 2 consecutive packets"
                                  : "confirmed by the build hint + 1 live packet";
            AC_LOG_INFO(kModule,
                        "CNetPacket layout = %s (m_pubData +0x%X, m_cubData +0x%X, "
                        "m_cRef +0x%X), %s after %d attempt(s).",
                        r.LayoutName(), r.DataOffset(), SizeOffFor(r.DataOffset()),
                        RefOffFor(r.DataOffset()), how, r.Attempts());
            diag::Record("netpacket_layout",
                         std::string(r.LayoutName()) + " dataOff=" +
                             std::to_string(r.DataOffset()) + " attempts=" +
                             std::to_string(r.Attempts()) + " confirmations=" +
                             std::to_string(r.Confirmations()));
        }
        return true;

    case Resolver::Step::Disabled:
        if (!g_disableLogged.exchange(true)) {
            AC_LOG_ERROR(kModule,
                         "CNetPacket layout unidentified after %d packets — wire features "
                         "are disabled for this session (no packet field will be touched). "
                         "The client's layout matches no known candidate: add a row to "
                         "ac::abi::netpkt::kLayouts (or publish it in the build's ABI table).",
                         kMaxProbeAttempts);
            diag::Record("netpacket_layout", "disabled after max attempts");
        }
        return false;

    case Resolver::Step::Ambiguous:
        AC_LOG_TRACE(kModule, "probe: several candidates matched, ambiguous (attempt %d).",
                     r.Attempts());
        return false;

    case Resolver::Step::AwaitingConfirm:
        AC_LOG_TRACE(kModule, "probe: candidate +0x%X matched, awaiting confirmation (attempt %d).",
                     r.Hinted(), r.Attempts());
        return false;

    case Resolver::Step::NoEvidence:
    default:
        return false;
    }
}

bool SeedFromAbiTable(std::uint32_t dataOff, std::uint32_t cubOff, std::uint32_t refOff,
                      const std::string& source) {
    // The accessors assume one shape: size eight bytes after the pointer,
    // refcount four after that. Every build disassembled so far agrees. If a
    // published table ever disagrees, the honest answer is not to "adapt" —
    // it is to refuse, shout, and let a human look, because the accessors
    // themselves would be wrong.
    if (cubOff != SizeOffFor(dataOff) || refOff != RefOffFor(dataOff)) {
        AC_LOG_ERROR(kModule,
                     "ABI table from %s describes m_pubData +0x%X / m_cubData +0x%X / "
                     "m_cRef +0x%X, which is not the shape this build of AetherCore knows "
                     "(+0x%X / +0x%X). Ignoring the table: the accessors would be wrong.",
                     source.c_str(), dataOff, cubOff, refOff, SizeOffFor(dataOff),
                     RefOffFor(dataOff));
        diag::Record("netpacket_layout", "abi table rejected: inconsistent shape");
        return false;
    }

    Resolver& r = Global();
    if (!r.AddCandidate(dataOff)) {
        AC_LOG_WARN(kModule, "ABI table candidate +0x%X not registered (table full).", dataOff);
        return false;
    }
    r.Hint(dataOff);
    g_hintSource.store("abi-table", std::memory_order_relaxed);
    AC_LOG_INFO(kModule,
                "ABI table (%s): CNetPacket m_pubData +0x%X — registered as a probe "
                "candidate; the live packet still decides.",
                source.c_str(), dataOff);
    diag::Record("netpacket_layout",
                 "abi table dataOff=" + std::to_string(dataOff) + " from " + source);
    return true;
}

const char* HintSource() { return g_hintSource.load(std::memory_order_relaxed); }

void SeedFromBuild(const std::string& steamclientSha256) {
    if (steamclientSha256.empty()) return;

    if (Global().Hinted() != kUnresolved) {
        // The per-build table already proposed a layout; the compiled list is
        // only a fallback for machines that have no table at all.
        return;
    }

    for (const auto& hint : kBuildHints) {
        if (strings::EqualsIgnoreCase(steamclientSha256, hint.sha256)) {
            Global().Hint(hint.dataOff);
            g_hintSource.store("build-hint", std::memory_order_relaxed);
            AC_LOG_INFO(kModule,
                        "Known build %s: CNetPacket hint m_pubData +0x%X (still probe-verified).",
                        hint.build, hint.dataOff);
            return;
        }
    }

    AC_LOG_INFO(kModule,
                "Unknown steamclient build (sha %.16s...): CNetPacket layout will be "
                "identified from live packets; wire features stay off until it is.",
                steamclientSha256.c_str());
    diag::Record("netpacket_layout", "unknown build, probe-only");
}

bool BeginWrite(const steam::CNetPacket* packet) {
    Resolver& r = Global();
    if (r.BeginWrite(packet, &DefaultReadable, nullptr)) return true;

    if (r.IsDisabled() && !g_disableLogged.exchange(true)) {
        AC_LOG_ERROR(kModule,
                     "CNetPacket write barrier refused %d consecutive packets with the latched "
                     "%s layout — the layout is wrong for this build. Wire features disabled; "
                     "no packet field will be touched again this session.",
                     kMaxWriteMismatches, r.LayoutName());
        diag::Record("netpacket_layout", "disabled by write barrier");
    } else {
        AC_LOG_TRACE(kModule, "write barrier: packet does not match the latched layout, skipped.");
    }
    return false;
}

const char* LayoutName() { return Global().LayoutName(); }
std::uint32_t ResolvedDataOffset() { return Global().DataOffset(); }
bool IsResolved() { return Global().IsResolved(); }
bool IsDisabled() { return Global().IsDisabled(); }
int ProbeAttempts() { return Global().Attempts(); }
int ProbeConfirmations() { return Global().Confirmations(); }
int WriteRejects() { return Global().WriteRejects(); }

}  // namespace ac::abi::netpkt
