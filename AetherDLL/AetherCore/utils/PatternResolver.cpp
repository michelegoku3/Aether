#include "pch.h"
#include "utils/PatternEngine.h"
#include <psapi.h>
#include <vector>
#include "core/AbiSentinel.h"
#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/SignatureCodec.h"
#pragma comment(lib, "Psapi.lib")

namespace ac::pattern {
    namespace {
        constexpr const char* kModule = "PatternEngine";
        using PatternIndex = AetherCoreState::PatternIndex;
        PatternIndex* IndexFor(const std::string& module) {
            if (module == "steamclient") return &g_state.patterns.steamclient;
            if (module == "steamui") return &g_state.patterns.steamui;
            return nullptr;
        }

        bool VerifySignature(const std::uint8_t* addr, const std::vector<std::uint8_t>& bytes,
            const std::string& mask) {
            for (std::size_t i = 0; i < bytes.size(); ++i) {
                if (mask[i] == 'x' && addr[i] != bytes[i]) return false;
            }
            return true;
        }

        // Looks up funcName inside the per-module name index.
        bool FindEntry(const PatternIndex& index, const std::string& funcName,
            std::string& rva, std::string& sig) {
            auto it = index.find(funcName);
            if (it == index.end()) return false;
            rva = it->second.rva;
            sig = it->second.sig;
            return !rva.empty();
        }

    }  // namespace

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
                AC_LOG_WARN(kModule, "'%s' not found in %s patterns.", funcName.c_str(),
                    module.c_str());
                return nullptr;
            }
            AC_LOG_DEBUG(kModule, "'%s' resolved via KeyValues_ alias.", funcName.c_str());
        }

        std::uintptr_t rva = 0;
        try {
            rva = std::stoull(rvaStr, nullptr, 16);
        }
        catch (...) {
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
                AC_LOG_WARN(kModule, "Malformed signature for '%s'; skipping hook for safety.",
                    funcName.c_str());
                return nullptr;
            }
            if (rva + bytes.size() > modInfo.SizeOfImage) {
                AC_LOG_WARN(kModule, "Signature for '%s' extends past module image; skipping hook.",
                    funcName.c_str());
                return nullptr;
            }
            if (!VerifySignature(target, bytes, mask)) {
                AC_LOG_WARN(kModule, "Signature mismatch for '%s'; skipping hook for safety.",
                    funcName.c_str());
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
        const auto verdict =
            abi::sentinel::Verify(funcName, target, hModule, &sentinelDetail);
        if (!abi::sentinel::Accepted(verdict)) {
            g_state.hookManager.RecordMissed(funcName, MissReason::SentinelRejected,
                                             sentinelDetail);
            return nullptr;
        }

        AC_LOG_DEBUG(kModule, "'%s' (%s) -> 0x%p", funcName.c_str(), module.c_str(),
            static_cast<void*>(target));
        return target;
    }

}  // namespace ac::pattern
