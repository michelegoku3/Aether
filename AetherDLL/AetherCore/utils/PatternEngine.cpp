#include "pch.h"
#include "utils/PatternEngine.h"

#include <psapi.h>
#include <toml++/toml.hpp>

#include <filesystem>
#include <fstream>
#include <sstream>
#include <unordered_map>
#include <vector>

#include "core/AetherCoreState.h"
#include "utils/Hasher.h"
#include "core/Logger.h"
#include "utils/PatternDownloader.h"
#include "utils/PatternFallbacks.h"

#pragma comment(lib, "Psapi.lib")

namespace ac::pattern {
    namespace {

        constexpr const char* kModule = "PatternEngine";

        using PatternIndex = AetherCoreState::PatternIndex;
        using PatternEntry = AetherCoreState::PatternEntry;

        PatternIndex* IndexFor(const std::string& module) {
            if (module == "steamclient") return &g_state.patterns.steamclient;
            if (module == "steamui") return &g_state.patterns.steamui;
            return nullptr;
        }

        void SetPatternStatus(const std::string& module, bool found, const std::string& source) {
            if (module == "steamclient") {
                g_state.steamclientTomlFound = found;
                g_state.steamclientPatternSource = source;
            }
            else if (module == "steamui") {
                g_state.steamuiTomlFound = found;
                g_state.steamuiPatternSource = source;
            }
        }

        bool BuildIndex(const toml::table& table, const std::string& moduleName, PatternIndex& outIndex) {
            outIndex.clear();
            for (const auto& [_, val] : table) {
                const auto* sub = val.as_table();
                if (!sub) continue;

                std::string name = sub->at_path("name").value_or(std::string{});
                std::string rva = sub->at_path("rva").value_or(std::string{});
                std::string sig = sub->at_path("sig").value_or(std::string{});
                if (name.empty() || rva.empty()) continue;

                auto [it, inserted] = outIndex.emplace(name, PatternEntry{ rva, sig });
                if (!inserted) {
                    AC_LOG_WARN(kModule, "Duplicate pattern name '%s' in %s; keeping first entry.",
                        name.c_str(), moduleName.c_str());
                    diag::Record("pattern_duplicate", moduleName + ":" + name);
                }
            }
            return !outIndex.empty();
        }

        // Fills gaps left by the remote table without allowing a fallback to replace a
        // build-specific TOML entry. This keeps all fallback knowledge in one file and
        // keeps hook modules independent from address data.
        bool MergeHardcodedFallbacks(const std::string& moduleName, PatternIndex& index) {
            bool added = false;
            for (const auto& fallback : kHardcodedPatterns) {
                if (fallback.module != moduleName) continue;
                if (index.find(std::string(fallback.name)) != index.end()) continue;

                index.emplace(std::string(fallback.name), PatternEntry{
                    std::string(fallback.rva), std::string(fallback.sig), true });
                added = true;
                AC_LOG_WARN(kModule,
                    "Using hardcoded fallback pattern for %s::%s (rva=%s%s).",
                    moduleName.c_str(), std::string(fallback.name).c_str(),
                    std::string(fallback.rva).c_str(),
                    fallback.sig.empty() ? ", signature absent" : ", signature verified");
                diag::Record("pattern_hardcoded_fallback",
                    moduleName + ":" + std::string(fallback.name));
            }
            return added;
        }

        bool ParsePatternFile(const std::string& path, const std::string& moduleName,
            toml::table& outTable, PatternIndex& outIndex) {
            std::ifstream file(path);
            if (!file.is_open()) return false;
            try {
                outTable = toml::parse(file);
                const bool indexed = BuildIndex(outTable, moduleName, outIndex);
                AC_LOG_INFO(kModule, "Loaded %zu TOML section(s), indexed %zu pattern name(s) for %s from %s.",
                    outTable.size(), outIndex.size(), moduleName.c_str(), path.c_str());
                return indexed;
            }
            catch (const toml::parse_error& err) {
                AC_LOG_ERROR(kModule, "TOML parse error for %s cache %s: %s", moduleName.c_str(),
                    path.c_str(), err.what());
                return false;
            }
        }

        void SweepTempFiles() {
            namespace fs = std::filesystem;
            std::error_code ec;
            if (!fs::is_directory(g_state.patternDir, ec)) return;
            for (const auto& entry : fs::directory_iterator(g_state.patternDir, ec)) {
                if (ec) break;
                if (!entry.is_regular_file(ec)) continue;
                const std::string name = entry.path().filename().string();
                if (name.size() >= 4 && name.compare(name.size() - 4, 4, ".tmp") == 0) {
                    fs::remove(entry.path(), ec);
                }
            }
        }

        // Loads the pattern table for one module. A corrupt cache is not fatal: it is
        // quarantined and the downloader gets one chance to refresh the TOML.
        bool LoadModule(const std::string& moduleName, const std::string& dllPath,
            std::string& outSha, PatternIndex& outIndex) {
            outIndex.clear();
            outSha = hasher::ComputeFileSha256(dllPath);
            if (outSha.empty()) {
                // A hardcoded fallback does not require a cache filename or network
                // lookup. Keep the feature available when hashing is unavailable, but
                // still let ResolveAddress apply module bounds/signature validation.
                const bool usedFallbacks = MergeHardcodedFallbacks(moduleName, outIndex);
                AC_LOG_ERROR(kModule,
                    "Could not hash %s; %s available.", dllPath.c_str(),
                    usedFallbacks ? "hardcoded fallbacks remain" : "no patterns remain");
                SetPatternStatus(moduleName, usedFallbacks, usedFallbacks ? "hash-failed+hardcoded" : "hash-failed");
                diag::Record(usedFallbacks ? "pattern_partial" : "pattern_missing", moduleName);
                return usedFallbacks;
            }
            AC_LOG_INFO(kModule, "%s SHA-256: %s", moduleName.c_str(), outSha.c_str());

            // toml::table is temporary — BuildIndex extracts everything into
            // PatternIndex, which is the only structure used at runtime.
            toml::table table;

            const std::string tomlPath = g_state.patternDir + "\\" + outSha + ".toml";
            if (GetFileAttributesA(tomlPath.c_str()) != INVALID_FILE_ATTRIBUTES) {
                if (ParsePatternFile(tomlPath, moduleName, table, outIndex)) {
                    const bool usedFallbacks = MergeHardcodedFallbacks(moduleName, outIndex);
                    SetPatternStatus(moduleName, true, usedFallbacks ? "cache+hardcoded" : "cache");
                    diag::Record("pattern", moduleName + ":" + (usedFallbacks ? "cache+hardcoded" : "cache"));
                    return true;
                }

                const std::string badPath = tomlPath + ".bad";
                MoveFileExA(tomlPath.c_str(), badPath.c_str(), MOVEFILE_REPLACE_EXISTING);
                AC_LOG_WARN(kModule, "Quarantined corrupt %s pattern cache as %s.",
                    moduleName.c_str(), badPath.c_str());
                diag::Record("pattern_cache_bad", moduleName);
            }
            else {
                AC_LOG_INFO(kModule, "No cached pattern for %s; downloading...", moduleName.c_str());
            }

            std::string downloadSource;
            if (!downloader::Download(moduleName, outSha, tomlPath, &downloadSource)) {
                const bool usedFallbacks = MergeHardcodedFallbacks(moduleName, outIndex);
                AC_LOG_WARN(kModule,
                    "Download failed for %s; %s available.", moduleName.c_str(),
                    usedFallbacks ? "hardcoded fallbacks remain" : "no patterns remain");
                SetPatternStatus(moduleName, usedFallbacks, usedFallbacks ? "missing+hardcoded" : "missing");
                diag::Record(usedFallbacks ? "pattern_partial" : "pattern_missing", moduleName);
                return usedFallbacks;
            }

            if (ParsePatternFile(tomlPath, moduleName, table, outIndex)) {
                const bool usedFallbacks = MergeHardcodedFallbacks(moduleName, outIndex);
                const std::string source = downloadSource.empty() ? "download" : downloadSource;
                const std::string sourceWithFallback = usedFallbacks ? source + "+hardcoded" : source;
                SetPatternStatus(moduleName, true, sourceWithFallback);
                diag::Record("pattern", moduleName + ":" + sourceWithFallback);
                return true;
            }

            DeleteFileA(tomlPath.c_str());
            const bool usedFallbacks = MergeHardcodedFallbacks(moduleName, outIndex);
            SetPatternStatus(moduleName, usedFallbacks ? true : false,
                usedFallbacks ? "invalid+hardcoded" : "invalid");
            diag::Record(usedFallbacks ? "pattern_partial" : "pattern_invalid", moduleName);
            AC_LOG_ERROR(kModule, "Downloaded pattern for %s is invalid; cache removed%s.",
                moduleName.c_str(), usedFallbacks ? "; hardcoded fallbacks retained" : "");
            return usedFallbacks;
        }

        // Parses a "AA ?? BB" signature string into bytes + mask. Returns false on
        // malformed input.
        bool ParseSignature(const std::string& sig, std::vector<std::uint8_t>& bytes, std::string& mask) {
            bytes.clear();
            mask.clear();
            std::istringstream iss(sig);
            std::string token;
            while (iss >> token) {
                if (token == "??") {
                    bytes.push_back(0);
                    mask.push_back('?');
                }
                else {
                    try {
                        if (token.size() != 2) {
                            AC_LOG_WARN(kModule, "Bad signature token '%s'.", token.c_str());
                            return false;
                        }
                        std::size_t consumed = 0;
                        unsigned long value = std::stoul(token, &consumed, 16);
                        if (consumed != token.size() || value > 0xFFul) {
                            AC_LOG_WARN(kModule, "Bad signature token '%s'.", token.c_str());
                            return false;
                        }
                        bytes.push_back(static_cast<std::uint8_t>(value));
                        mask.push_back('x');
                    }
                    catch (...) {
                        AC_LOG_WARN(kModule, "Bad signature token '%s'.", token.c_str());
                        return false;
                    }
                }
            }
            return !bytes.empty();
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
            std::string& rva, std::string& sig, bool& hardcodedFallback) {
            auto it = index.find(funcName);
            if (it == index.end()) return false;
            rva = it->second.rva;
            sig = it->second.sig;
            hardcodedFallback = it->second.hardcodedFallback;
            return !rva.empty();
        }

    }  // namespace

    bool Init() {
        AC_LOG_INFO(kModule, "Initialising.");
        CreateDirectoryA(g_state.patternDir.c_str(), nullptr);
        SweepTempFiles();

        bool steamclientOk = LoadModule("steamclient", g_state.steamclientPath,
            g_state.steamclientSha, g_state.patterns.steamclient);

        g_state.steamuiPath = g_state.steamInstallPath + "\\steamui.dll";
        bool steamuiOk = LoadModule("steamui", g_state.steamuiPath,
            g_state.steamuiSha, g_state.patterns.steamui);

        return steamclientOk || steamuiOk;
    }

    void* ResolveAddress(const std::string& funcName, const std::string& module, HMODULE hModule) {
        PatternIndex* index = IndexFor(module);
        if (!index) {
            AC_LOG_WARN(kModule, "Unknown module '%s'.", module.c_str());
            return nullptr;
        }

        std::string rvaStr, sigStr;
        bool hardcodedFallback = false;
        if (!FindEntry(*index, funcName, rvaStr, sigStr, hardcodedFallback)) {
            // Some publishers prefix KeyValues helpers as "KeyValues_<name>". Retry
            // with that alias before giving up.
            if (!FindEntry(*index, "KeyValues_" + funcName, rvaStr, sigStr, hardcodedFallback)) {
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

        if (hardcodedFallback && sigStr.empty()) {
            AC_LOG_WARN(kModule,
                "'%s' uses a hardcoded RVA-only fallback; no signature is available.",
                funcName.c_str());
            diag::Record("pattern_unverified_fallback", module + ":" + funcName);
        }

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

        AC_LOG_DEBUG(kModule, "'%s' (%s%s) -> 0x%p", funcName.c_str(), module.c_str(),
            hardcodedFallback ? "+hardcoded" : "", static_cast<void*>(target));
        return target;
    }

}  // namespace ac::pattern
