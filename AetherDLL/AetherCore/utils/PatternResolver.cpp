#include "pch.h"
#include "core/AbiSentinel.h"
#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/PatternEngine.h"
#include "utils/SignatureCodec.h"
#include "utils/SignatureScan.h"
#include <algorithm>
#include <psapi.h>
#include <vector>
#pragma comment(lib, "Psapi.lib")

namespace ac::pattern {
namespace {
constexpr const char* kModule = "PatternEngine";
using PatternIndex = AetherCoreState::PatternIndex;
PatternIndex* IndexFor(const std::string& module) {
    if (module == "steamclient")
        return &g_state.patterns.steamclient;
    if (module == "steamui")
        return &g_state.patterns.steamui;
    return nullptr;
}

bool VerifySignature(const std::uint8_t* addr, const std::vector<std::uint8_t>& bytes, const std::string& mask) {
    for (std::size_t i = 0; i < bytes.size(); ++i) {
        if (mask[i] == 'x' && addr[i] != bytes[i])
            return false;
    }
    return true;
}

// Looks up funcName inside the per-module name index.
bool FindEntry(const PatternIndex& index, const std::string& funcName, std::string& rva, std::string& sig) {
    auto it = index.find(funcName);
    if (it == index.end())
        return false;
    rva = it->second.rva;
    sig = it->second.sig;
    return !rva.empty();
}

} // namespace

void* ResolveAddress(const std::string& funcName, const std::string& module, HMODULE hModule) {
    std::shared_lock lock(g_state.patterns.mutex);
    PatternIndex* index = IndexFor(module);
    if (!index) {
        AC_LOG_WARN(kModule, "Unknown module '%s'.", module.c_str());
        return nullptr;
    }

    std::string rvaStr, sigStr;
    if (!FindEntry(*index, funcName, rvaStr, sigStr)) {
        // Some publishers prefix KeyValues helpers as "KeyValues_<name>". Retry
        // with that alias before giving up.
        if (!FindEntry(*index, "KeyValues_" + funcName, rvaStr, sigStr)) {
            AC_LOG_WARN(kModule, "'%s' not found in %s patterns.", funcName.c_str(), module.c_str());
            return nullptr;
        }
        AC_LOG_DEBUG(kModule, "'%s' resolved via KeyValues_ alias.", funcName.c_str());
    }

    std::uintptr_t rva = 0;
    try {
        rva = std::stoull(rvaStr, nullptr, 16);
    } catch (...) {
        AC_LOG_WARN(kModule, "Bad RVA '%s' for %s.", rvaStr.c_str(), funcName.c_str());
        return nullptr;
    }

    // Bounds-check the RVA against the real loaded image rather than trusting
    // it blindly (avoids IsBadReadPtr-style faults if the TOML is stale).
    MODULEINFO modInfo{};
    if (!GetModuleInformation(GetCurrentProcess(), hModule, &modInfo, sizeof(modInfo))) {
        AC_LOG_ERROR(kModule, "GetModuleInformation failed for %s.", module.c_str());
        return nullptr;
    }
    if (rva >= modInfo.SizeOfImage) {
        AC_LOG_ERROR(kModule, "RVA 0x%zx out of range for '%s'.", rva, funcName.c_str());
        return nullptr;
    }

    auto* target = reinterpret_cast<std::uint8_t*>(modInfo.lpBaseOfDll) + rva;

    if (!sigStr.empty()) {
        std::vector<std::uint8_t> bytes;
        std::string mask;
        if (!ParseSignature(sigStr, bytes, mask)) {
            AC_LOG_WARN(kModule, "Malformed signature for '%s'; skipping hook for safety.", funcName.c_str());
            return nullptr;
        }
        if (rva + bytes.size() > modInfo.SizeOfImage) {
            AC_LOG_WARN(kModule, "Signature for '%s' extends past module image; skipping hook.", funcName.c_str());
            return nullptr;
        }
        if (!VerifySignature(target, bytes, mask)) {
            AC_LOG_WARN(kModule, "Signature mismatch for '%s'; skipping hook for safety.", funcName.c_str());
            return nullptr;
        }
    }

    // Last gate before anything is hooked: the signature proves the BYTES
    // match, the sentinel proves the ADDRESS is a function entry. They
    // catch different defects — a table can carry a correct signature for
    // an RVA that points a few bytes into the function (it happened with
    // the 0x5BC460 pin for RecvPkt), and MinHook would then overwrite live
    // instructions mid-block.
    std::string sentinelDetail;
    const auto verdict = abi::sentinel::Verify(funcName, target, hModule, &sentinelDetail);
    if (!abi::sentinel::Accepted(verdict)) {
        g_state.hookManager.RecordMissed(funcName, MissReason::SentinelRejected, sentinelDetail);
        return nullptr;
    }

    AC_LOG_DEBUG(kModule, "'%s' (%s) -> 0x%p", funcName.c_str(), module.c_str(), static_cast<void*>(target));
    return target;
}

void* ResolveOrScanAddress(const std::string& funcName, const std::string& module, HMODULE hModule,
                           const char* fallbackSig, bool* resolvedByScan) {
    if (resolvedByScan) {
        *resolvedByScan = false;
    }

    bool tableHasEntry = false;
    {
        std::shared_lock lock(g_state.patterns.mutex);
        if (PatternIndex* index = IndexFor(module)) {
            std::string rvaStr, sigStr;
            tableHasEntry = FindEntry(*index, funcName, rvaStr, sigStr) ||
                            FindEntry(*index, "KeyValues_" + funcName, rvaStr, sigStr);
        }
    }

    if (tableHasEntry) {
        return ResolveAddress(funcName, module, hModule);
    }

    if (!fallbackSig || fallbackSig[0] == '\0' || !hModule) {
        AC_LOG_WARN(kModule, "'%s' not found in %s patterns (no fallback signature).", funcName.c_str(),
                    module.c_str());
        g_state.hookManager.RecordMissed(funcName, MissReason::PatternUnresolved,
                                         "missing from pattern table and no fallback signature");
        return nullptr;
    }

    std::vector<std::uint8_t> bytes;
    std::string mask;
    if (!ParseSignature(fallbackSig, bytes, mask)) {
        AC_LOG_WARN(kModule, "Malformed fallback signature for '%s'.", funcName.c_str());
        g_state.hookManager.RecordMissed(funcName, MissReason::PatternUnresolved, "malformed fallback signature");
        return nullptr;
    }

    MODULEINFO modInfo{};
    if (!GetModuleInformation(GetCurrentProcess(), hModule, &modInfo, sizeof(modInfo))) {
        AC_LOG_ERROR(kModule, "GetModuleInformation failed for %s.", module.c_str());
        return nullptr;
    }

    const auto* modBase = static_cast<const std::uint8_t*>(modInfo.lpBaseOfDll);
    if (!modBase || modInfo.SizeOfImage < sizeof(IMAGE_DOS_HEADER)) {
        return nullptr;
    }

    const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(modBase);
    if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew <= 0 ||
        static_cast<std::size_t>(dos->e_lfanew) + sizeof(IMAGE_NT_HEADERS) > modInfo.SizeOfImage) {
        AC_LOG_WARN(kModule, "Invalid PE headers while scanning %s for '%s'.", module.c_str(), funcName.c_str());
        return nullptr;
    }

    const auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS*>(modBase + dos->e_lfanew);
    if (nt->Signature != IMAGE_NT_SIGNATURE) {
        AC_LOG_WARN(kModule, "Invalid NT signature while scanning %s for '%s'.", module.c_str(), funcName.c_str());
        return nullptr;
    }

    const auto* sections = IMAGE_FIRST_SECTION(nt);
    const std::size_t secEnd = static_cast<std::size_t>(
        reinterpret_cast<const std::uint8_t*>(sections + nt->FileHeader.NumberOfSections) - modBase);
    if (secEnd > modInfo.SizeOfImage) {
        return nullptr;
    }

    const std::uint8_t* uniqueMatch = nullptr;
    for (WORD i = 0; i < nt->FileHeader.NumberOfSections; ++i) {
        const auto& sec = sections[i];
        if ((sec.Characteristics & IMAGE_SCN_MEM_EXECUTE) == 0) {
            continue;
        }
        const std::size_t va = sec.VirtualAddress;
        const std::size_t vsize = sec.Misc.VirtualSize;
        if (va >= modInfo.SizeOfImage || vsize == 0) {
            continue;
        }
        const std::size_t scanLen = (std::min)(vsize, static_cast<std::size_t>(modInfo.SizeOfImage) - va);
        const ScanResult res = FindUniqueSignatureMatch(modBase + va, scanLen, bytes, mask);
        if (res.status == ScanStatus::Ambiguous) {
            AC_LOG_WARN(kModule, "Fallback signature for '%s' in %s is ambiguous; skipping.", funcName.c_str(),
                        module.c_str());
            g_state.hookManager.RecordMissed(funcName, MissReason::PatternUnresolved,
                                             "fallback signature matched multiple locations");
            return nullptr;
        }
        if (res.status == ScanStatus::Unique) {
            if (uniqueMatch != nullptr) {
                AC_LOG_WARN(kModule, "Fallback signature for '%s' in %s matched across sections; skipping.",
                            funcName.c_str(), module.c_str());
                g_state.hookManager.RecordMissed(funcName, MissReason::PatternUnresolved,
                                                 "fallback signature matched multiple sections");
                return nullptr;
            }
            uniqueMatch = modBase + va + res.offset;
        }
    }

    if (!uniqueMatch) {
        AC_LOG_WARN(kModule, "Fallback signature for '%s' not found in executable sections of %s.", funcName.c_str(),
                    module.c_str());
        g_state.hookManager.RecordMissed(funcName, MissReason::PatternUnresolved,
                                         "fallback signature not found in module");
        return nullptr;
    }

    auto* target = const_cast<std::uint8_t*>(uniqueMatch);
    std::string sentinelDetail;
    const auto verdict = abi::sentinel::Verify(funcName, target, hModule, &sentinelDetail);
    if (!abi::sentinel::Accepted(verdict)) {
        g_state.hookManager.RecordMissed(funcName, MissReason::SentinelRejected, sentinelDetail);
        return nullptr;
    }

    if (resolvedByScan) {
        *resolvedByScan = true;
    }
    AC_LOG_INFO(kModule, "'%s' (%s) resolved via signature scan -> 0x%p", funcName.c_str(), module.c_str(),
                static_cast<void*>(target));
    return target;
}

} // namespace ac::pattern
