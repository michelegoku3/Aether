#include "pch.h"
#include "utils/PatternEngine.h"

#include <psapi.h>
#include <toml++/toml.hpp>

#include <filesystem>
#include <fstream>
#include <sstream>
#include <thread>
#include <unordered_map>
#include <vector>

#include "core/AetherCoreState.h"
#include "utils/Hasher.h"
#include "core/Logger.h"
#include "utils/PatternDownloader.h"

#pragma comment(lib, "Psapi.lib")

namespace ac::pattern {
    namespace {

        constexpr const char* kModule = "PatternEngine";

        // Per-HTTP-attempt cap for the boot-time provenance probe: an
        // unresponsive network must not stall pattern resolution for long
        // (the cached table is perfectly usable while the check times out).
        constexpr int kUpgradeProbeTimeoutSec = 5;

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

        // ---- Provenance sidecar -------------------------------------------
        // Every cache TOML is paired with a "<sha>.toml.src" sidecar holding the
        // exact source label that produced the file (e.g. "migo:raw"). On a
        // later launch the sidecar lets the loader decide, without any network
        // traffic, that the cached table already comes from the highest-priority
        // source and needs no refresh.

        std::string ReadProvenance(const std::string& srcPath) {
            std::ifstream in(srcPath);
            if (!in) return {};
            std::string label;
            std::getline(in, label);
            while (!label.empty() && (label.back() == ' ' || label.back() == '\t' ||
                                      label.back() == '\r' || label.back() == '\n')) {
                label.pop_back();
            }
            return label;
        }

        void WriteProvenance(const std::string& srcPath, const std::string& label) {
            const std::string tmp = srcPath + ".tmp";
            {
                std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
                if (!out.is_open()) return;
                out << label;
            }
            if (!MoveFileExA(tmp.c_str(), srcPath.c_str(), MOVEFILE_REPLACE_EXISTING)) {
                DeleteFileA(tmp.c_str());
            }
        }

        // The source a fresh download would be expected to carry for this module
        // right now: the configured user mirror if any, else the first source in
        // the registry that carries the kind. Empty when nothing can serve it.
        std::string BestExpectedSource(const std::string& moduleName) {
            if (!g_state.settings.patternMirror.empty()) return "mirror";
            const downloader::Kind kind = downloader::KindFromName(moduleName);
            for (const downloader::Source& src : downloader::DefaultSources()) {
                if (src.LocFor(kind) != nullptr) return std::string(src.id);
            }
            return {};
        }

        // Sidecar labels are "mirror" or "<source>:<mirror>" (e.g. "migo:raw").
        // A label is "best" when its source part matches the current
        // first-priority source for the module. Unknown provenance (missing
        // sidecar, e.g. caches written by builds without sidecar support) is NOT
        // best: the upstream must be consulted to decide whether an upgrade now
        // exists.
        bool ProvenanceIsBest(const std::string& provenance, const std::string& expected) {
            if (expected.empty()) return true;  // no better source exists
            if (provenance == "mirror") return expected == "mirror";
            const std::size_t colon = provenance.find(':');
            const std::string source = (colon == std::string::npos)
                ? provenance : provenance.substr(0, colon);
            return source == expected;
        }

        // Number of entries a TOML file would index. Used to refuse an "upgrade"
        // that would shrink the table we already have.
        std::size_t CountPatternEntries(const std::string& path) {
            try {
                std::ifstream file(path);
                if (!file.is_open()) return 0;
                const toml::table parsed = toml::parse(file);
                std::size_t count = 0;
                for (const auto& [_, val] : parsed) {
                    const auto* sub = val.as_table();
                    if (!sub) continue;
                    if (sub->at_path("name").value_or(std::string{}).empty()) continue;
                    if (sub->at_path("rva").value_or(std::string{}).empty()) continue;
                    ++count;
                }
                return count;
            }
            catch (...) {
                return 0;
            }
        }

        // Loads the pattern table for one module. A corrupt cache is not fatal:
        // it is quarantined and the downloader gets one chance to refresh the
        // TOML.
        //
        // Source policy (per-launch refresh, upgrade-only):
        //   * a valid local cache is always usable immediately;
        //   * if its provenance sidecar says the table already came from the
        //     highest-priority source, the cache is served as-is, no network;
        //   * otherwise (missing/older sidecar, or a lower-priority source such
        //     as OpenSteamTool while MigoReleases has now published the build)
        //     the upstream chain is consulted in priority order. The winner
        //     replaces the cache ONLY when it is not smaller than the cached
        //     table -- coverage is never downgraded. The sidecar is rewritten so
        //     the following launch is network-free.
        bool LoadModule(const std::string& moduleName, const std::string& dllPath,
            std::string& outSha, PatternIndex& outIndex) {
            outIndex.clear();
            outSha = hasher::ComputeFileSha256(dllPath);
            if (outSha.empty()) {
                // Without a SHA we cannot name the per-build cache file or fetch the
                // correct table: address resolution is unavailable for this module.
                AC_LOG_ERROR(kModule, "Could not hash %s; no patterns available.", dllPath.c_str());
                SetPatternStatus(moduleName, false, "hash-failed");
                diag::Record("pattern_missing", moduleName);
                return false;
            }
            AC_LOG_INFO(kModule, "%s SHA-256: %s", moduleName.c_str(), outSha.c_str());

            // toml::table is temporary -- BuildIndex extracts everything into
            // PatternIndex, which is the only structure used at runtime.
            toml::table table;

            const std::string tomlPath = g_state.patternDir + "\\" + outSha + ".toml";
            const std::string srcPath = tomlPath + ".src";
            const std::string expectedBest = BestExpectedSource(moduleName);

            // Local cache: parse it when present (a failure is quarantined and
            // triggers the download path below).
            bool cacheUsable = false;
            std::string refreshedFrom;  // set when an upstream table is adopted
            if (GetFileAttributesA(tomlPath.c_str()) != INVALID_FILE_ATTRIBUTES) {
                cacheUsable = ParsePatternFile(tomlPath, moduleName, table, outIndex);
                if (!cacheUsable) {
                    const std::string badPath = tomlPath + ".bad";
                    MoveFileExA(tomlPath.c_str(), badPath.c_str(), MOVEFILE_REPLACE_EXISTING);
                    DeleteFileA(srcPath.c_str());  // stale provenance dies with the file
                    AC_LOG_WARN(kModule, "Quarantined corrupt %s pattern cache as %s.",
                        moduleName.c_str(), badPath.c_str());
                    diag::Record("pattern_cache_bad", moduleName);
                }
            }
            else {
                AC_LOG_INFO(kModule, "No cached pattern for %s; downloading...", moduleName.c_str());
            }

            if (cacheUsable && !ProvenanceIsBest(ReadProvenance(srcPath), expectedBest)) {
                // Local table exists but is not known to come from the preferred
                // source: probe the upstream chain (upgrade-only policy).
                const std::string provenance = ReadProvenance(srcPath);
                AC_LOG_INFO(kModule, "Cached %s patterns (provenance '%s') are not from the "
                                     "preferred source ('%s'); checking upstream.",
                            moduleName.c_str(),
                            provenance.empty() ? "<unknown>" : provenance.c_str(),
                            expectedBest.c_str());

                // Download to a temp name: the existing cache must never be
                // clobbered before the candidate has been inspected.
                const std::string candidatePath = tomlPath + ".tmp";
                std::string fetched;
                if (!downloader::Download(moduleName, outSha, candidatePath, &fetched,
                                          kUpgradeProbeTimeoutSec)) {
                    AC_LOG_INFO(kModule, "No upstream source can serve %s right now; "
                                         "keeping the cached table.", moduleName.c_str());
                }
                else if (CountPatternEntries(candidatePath) >= outIndex.size()) {
                    toml::table fresh;
                    PatternIndex freshIndex;
                    if (ParsePatternFile(candidatePath, moduleName, fresh, freshIndex)) {
                        if (MoveFileExA(candidatePath.c_str(), tomlPath.c_str(),
                                        MOVEFILE_REPLACE_EXISTING)) {
                            WriteProvenance(srcPath, fetched);
                            outIndex.swap(freshIndex);
                            table = std::move(fresh);
                            refreshedFrom = fetched;
                            AC_LOG_INFO(kModule, "Upgraded %s patterns from '%s' (%zu entries).",
                                        moduleName.c_str(), fetched.c_str(), outIndex.size());
                        }
                        else {
                            DeleteFileA(candidatePath.c_str());
                            AC_LOG_WARN(kModule, "Could not adopt upstream %s patterns; "
                                                 "keeping the cached table.", moduleName.c_str());
                        }
                    }
                    else {
                        DeleteFileA(candidatePath.c_str());
                        AC_LOG_WARN(kModule, "Upstream %s candidate could not be parsed; "
                                             "keeping the cached table.", moduleName.c_str());
                    }
                }
                else {
                    DeleteFileA(candidatePath.c_str());
                    AC_LOG_INFO(kModule, "Upstream '%s' candidate for %s is smaller than the "
                                         "cached table; keeping the cache.",
                                fetched.c_str(), moduleName.c_str());
                }
            }

            if (cacheUsable) {
                // Served from the local cache: it already had the best provenance
                // (no network spent) or the upgrade attempt above found nothing
                // better to replace it with. The label reports the real origin:
                // refreshed tables carry the plain fetch label; cached tables the
                // provenance from the sidecar; provenance-less caches stay "cache".
                const std::string provenance = ReadProvenance(srcPath);
                std::string source;
                if (!refreshedFrom.empty()) {
                    source = refreshedFrom;
                }
                else if (!provenance.empty()) {
                    source = provenance + " (cache)";
                }
                else {
                    source = "cache";
                }
                SetPatternStatus(moduleName, true, source);
                diag::Record("pattern", moduleName + ":" + source);
                return true;
            }

            std::string downloadSource;
            if (!downloader::Download(moduleName, outSha, tomlPath, &downloadSource)) {
                AC_LOG_WARN(kModule, "Download failed for %s; no patterns available.", moduleName.c_str());
                SetPatternStatus(moduleName, false, "missing");
                diag::Record("pattern_missing", moduleName);
                return false;
            }

            if (ParsePatternFile(tomlPath, moduleName, table, outIndex)) {
                const std::string source = downloadSource.empty() ? "download" : downloadSource;
                WriteProvenance(srcPath, source);
                SetPatternStatus(moduleName, true, source);
                diag::Record("pattern", moduleName + ":" + source);
                return true;
            }

            DeleteFileA(tomlPath.c_str());
            DeleteFileA(srcPath.c_str());
            SetPatternStatus(moduleName, false, "invalid");
            diag::Record("pattern_invalid", moduleName);
            AC_LOG_ERROR(kModule, "Downloaded pattern for %s is invalid; cache removed.",
                moduleName.c_str());
            return false;
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
            std::string& rva, std::string& sig) {
            auto it = index.find(funcName);
            if (it == index.end()) return false;
            rva = it->second.rva;
            sig = it->second.sig;
            return !rva.empty();
        }

    }  // namespace

    bool Init() {
        AC_LOG_INFO(kModule, "Initialising.");
        CreateDirectoryA(g_state.patternDir.c_str(), nullptr);
        SweepTempFiles();

        g_state.steamuiPath = g_state.steamInstallPath + "\\steamui.dll";

        // Resolve both modules concurrently. On a fresh Steam build every cache
        // miss costs network round-trips; running them in parallel cuts the
        // critical path before hook installation in half (steamui's resolution
        // no longer waits behind steamclient's), so the hooks are in place
        // before Steam fires the one-shot LoadPackage(package 0) at startup.
        //
        // The two threads touch disjoint pattern indexes. They compute their
        // SHA into LOCAL strings on purpose: dllmain publishes
        // g_state.steamclientSha before spawning the concurrent IPC-spec
        // resolution, which reads it — writing the std::string here while that
        // thread reads it would be a data race. The local steamclient SHA is
        // identical to the published one; only steamuiSha is owned by Init.
        bool steamclientOk = false;
        bool steamuiOk = false;
        std::string clientSha;
        std::string uiSha;
        std::thread clientThread([&] {
            steamclientOk = LoadModule("steamclient", g_state.steamclientPath,
                clientSha, g_state.patterns.steamclient);
        });
        std::thread uiThread([&] {
            steamuiOk = LoadModule("steamui", g_state.steamuiPath,
                uiSha, g_state.patterns.steamui);
        });
        clientThread.join();
        uiThread.join();

        // Published only after both threads join: no concurrent reader exists at
        // this point (the parallel IPC resolution reads steamclientSha, which is
        // owned by dllmain and deliberately left untouched above).
        g_state.steamuiSha = uiSha;

        return steamclientOk || steamuiOk;
    }

    bool ReloadModuleIfMissing(const std::string& module) {
        PatternIndex* index = IndexFor(module);
        if (!index) return false;
        if (!index->empty()) return true;  // already loaded this session

        const std::string* dllPath = (module == "steamui") ? &g_state.steamuiPath
                                                           : &g_state.steamclientPath;
        if (!dllPath || dllPath->empty()) return false;

        // LoadModule applies the provenance policy itself (see above): a cached
        // table is served without network when its sidecar says it already came
        // from the preferred source, and the upstream chain is consulted
        // otherwise. Re-running it here therefore picks up both a table that
        // arrived after init and, at most once per module, an upstream upgrade.
        std::string sha;
        if (!LoadModule(module, *dllPath, sha, *index)) return false;
        if (module == "steamui") g_state.steamuiSha = sha;
        return !index->empty();
    }

    void* ResolveAddress(const std::string& funcName, const std::string& module, HMODULE hModule) {
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

        AC_LOG_DEBUG(kModule, "'%s' (%s) -> 0x%p", funcName.c_str(), module.c_str(),
            static_cast<void*>(target));
        return target;
    }

}  // namespace ac::pattern
