#include "pch.h"
#include "core/AbiSentinel.h"

#include <windows.h>
#include <psapi.h>

#include <algorithm>
#include <cstring>
#include <vector>

#include "core/Logger.h"

namespace ac::abi::sentinel {
namespace {

constexpr const char* kModule = "Abi.Sentinel";

std::atomic<std::uint32_t> g_verified{0};
std::atomic<std::uint32_t> g_rejected{0};

// Reads `bytes` from the live image only when the whole range is committed
// and readable. The module is mapped and we are about to hook it, so this is
// belt and braces — but a stale RVA near the end of a section is exactly the
// case that would fault here instead of being reported.
bool SafeRead(const void* addr, void* out, std::size_t bytes) {
    MEMORY_BASIC_INFORMATION mbi{};
    if (::VirtualQuery(addr, &mbi, sizeof(mbi)) != sizeof(mbi)) return false;
    if (mbi.State != MEM_COMMIT) return false;
    if ((mbi.Protect & (PAGE_NOACCESS | PAGE_GUARD)) != 0 || mbi.Protect == 0) return false;
    const auto start = reinterpret_cast<std::uintptr_t>(addr);
    const auto regionEnd = reinterpret_cast<std::uintptr_t>(mbi.BaseAddress) + mbi.RegionSize;
    if (start + bytes > regionEnd) return false;  // straddles regions: refuse, do not guess
    std::memcpy(out, addr, bytes);
    return true;
}

struct ImageView {
    const std::uint8_t* base = nullptr;
    std::uint32_t sizeOfImage = 0;
    std::uint32_t codeStart = 0;
    std::uint32_t codeEnd = 0;
    std::vector<PdataEntry> pdata;  // sorted by begin (PE guarantees it; we sort anyway)
    bool valid = false;
};

// Parses just enough of the mapped PE to find the executable range and the
// exception directory. Deliberately hand-rolled: pulling in DbgHelp for this
// would add a dependency and a loader-lock hazard.
ImageView Parse(void* module) {
    ImageView v;
    if (!module) return v;

    MODULEINFO info{};
    if (!::GetModuleInformation(::GetCurrentProcess(), static_cast<HMODULE>(module), &info,
                                sizeof(info))) {
        return v;
    }
    v.base = static_cast<const std::uint8_t*>(info.lpBaseOfDll);
    v.sizeOfImage = info.SizeOfImage;

    const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(v.base);
    if (dos->e_magic != IMAGE_DOS_SIGNATURE) return v;
    const auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS64*>(v.base + dos->e_lfanew);
    if (nt->Signature != IMAGE_NT_SIGNATURE) return v;

    // Executable range: union of the sections marked executable. Steam ships a
    // single .text, but a union costs nothing and survives a second one.
    const auto* sec = IMAGE_FIRST_SECTION(nt);
    for (unsigned i = 0; i < nt->FileHeader.NumberOfSections; ++i, ++sec) {
        if ((sec->Characteristics & IMAGE_SCN_MEM_EXECUTE) == 0) continue;
        const std::uint32_t start = sec->VirtualAddress;
        const std::uint32_t end = start + sec->Misc.VirtualSize;
        if (v.codeEnd == 0) {
            v.codeStart = start;
            v.codeEnd = end;
        } else {
            if (start < v.codeStart) v.codeStart = start;
            if (end > v.codeEnd) v.codeEnd = end;
        }
    }

    const IMAGE_DATA_DIRECTORY& dir =
        nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION];
    if (dir.VirtualAddress != 0 && dir.Size >= sizeof(RUNTIME_FUNCTION) &&
        dir.VirtualAddress + dir.Size <= v.sizeOfImage) {
        const auto* rf = reinterpret_cast<const RUNTIME_FUNCTION*>(v.base + dir.VirtualAddress);
        const std::size_t n = dir.Size / sizeof(RUNTIME_FUNCTION);
        v.pdata.reserve(n);
        for (std::size_t i = 0; i < n; ++i) {
            if (rf[i].BeginAddress == 0 || rf[i].EndAddress <= rf[i].BeginAddress) continue;
            v.pdata.push_back(PdataEntry{rf[i].BeginAddress, rf[i].EndAddress});
        }
        std::sort(v.pdata.begin(), v.pdata.end(),
                  [](const PdataEntry& a, const PdataEntry& b) { return a.begin < b.begin; });
    }

    v.valid = v.codeEnd > v.codeStart;
    return v;
}

}  // namespace

const char* VerdictText(Verdict v) {
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

Verdict Verify(const std::string& name, const void* target, void* module, std::string* detail) {
    const ImageView view = Parse(module);
    if (!view.valid || !target) {
        if (detail) *detail = "module image could not be parsed";
        g_rejected.fetch_add(1, std::memory_order_relaxed);
        AC_LOG_WARN(kModule, "'%s': module image could not be parsed; hook skipped.",
                    name.c_str());
        return Verdict::OutsideCode;
    }

    const auto* addr = static_cast<const std::uint8_t*>(target);
    if (addr < view.base || addr >= view.base + view.sizeOfImage) {
        if (detail) *detail = "target outside the module";
        g_rejected.fetch_add(1, std::memory_order_relaxed);
        AC_LOG_WARN(kModule, "'%s': target outside the module; hook skipped.", name.c_str());
        return Verdict::OutsideCode;
    }

    const auto rva = static_cast<std::uint32_t>(addr - view.base);
    const bool inCode = rva >= view.codeStart && rva < view.codeEnd;

    std::uint8_t prevByte = 0;
    if (rva == 0 || !SafeRead(addr - 1, &prevByte, 1)) prevByte = 0xFF;  // never a boundary
    std::uint8_t first[8]{};
    const bool haveBytes = SafeRead(addr, first, sizeof(first));

    const Verdict v = Classify(rva, inCode, view.pdata.data(), view.pdata.size(), prevByte,
                               haveBytes ? first : nullptr, haveBytes ? sizeof(first) : 0);

    if (Accepted(v)) {
        g_verified.fetch_add(1, std::memory_order_relaxed);
        AC_LOG_DEBUG(kModule, "'%s' rva 0x%X: %s.", name.c_str(), rva, VerdictText(v));
        return v;
    }

    g_rejected.fetch_add(1, std::memory_order_relaxed);
    if (detail) *detail = VerdictText(v);
    // ERROR, not WARN: a pattern table that points into the middle of a
    // function is the failure mode that corrupts Steam silently. The operator
    // must see it even when skimming.
    AC_LOG_ERROR(kModule,
                 "'%s' rva 0x%X REJECTED: %s (prev byte 0x%02X, first bytes %02X %02X %02X %02X). "
                 "Hook skipped — the pattern table for this build is wrong for this entry.",
                 name.c_str(), rva, VerdictText(v), prevByte, first[0], first[1], first[2],
                 first[3]);
    diag::Record("abi_sentinel", name + ": " + VerdictText(v));
    return v;
}

std::uint32_t VerifiedCount() { return g_verified.load(std::memory_order_relaxed); }
std::uint32_t RejectedCount() { return g_rejected.load(std::memory_order_relaxed); }

void ResetCounters() {
    g_verified.store(0, std::memory_order_relaxed);
    g_rejected.store(0, std::memory_order_relaxed);
}

}  // namespace ac::abi::sentinel
