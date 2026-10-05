#include "pch.h"
#include "utils/PatternCache.h"
#include <filesystem>
#include <fstream>
#include <unordered_map>
#include <toml++/toml.hpp>
#include "core/Logger.h"
#include "utils/Hasher.h"
#include "utils/PatternDownloader.h"

namespace ac::pattern::detail {
    namespace {
        constexpr const char* kModule = "PatternEngine";
        // Bound the optional upgrade probe; a valid cache remains usable.
        constexpr int kUpgradeProbeTimeoutSec = 5;
        using PatternEntry = AetherCoreState::PatternEntry;

        void SetPatternStatus(const std::string& module, bool found, const std::string& source) {
            std::lock_guard lock(g_state.statusMetadataMutex);
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

        // Reads the `[abi.<Struct>.<field>]` sections a phase-4 table carries.
        // Absent on older tables, which is why nothing here is fatal: a build
        // without an ABI section simply falls back to the probe.
        void CollectAbiTable(const toml::table& table, const std::string& moduleName,
                             const std::string& sha) {
            if (moduleName != "steamclient") return;
            const auto* abi = table["abi"].as_table();
            if (!abi) return;

            std::unordered_map<std::string, std::uint32_t> offsets;
            for (const auto& [structKey, structNode] : *abi) {
                const auto* fields = structNode.as_table();
                if (!fields) continue;
                for (const auto& [fieldKey, fieldNode] : *fields) {
                    const auto* entry = fieldNode.as_table();
                    if (!entry) continue;
                    const auto value = (*entry)["offset"].value<std::int64_t>();
                    if (!value || *value < 0 || *value > 0xFFFF) continue;
                    offsets.emplace(std::string(structKey.str()) + "." +
                                        std::string(fieldKey.str()),
                                    static_cast<std::uint32_t>(*value));
                }
            }
            if (offsets.empty()) return;

            {
                std::lock_guard lock(g_state.abiTable.mutex);
                g_state.abiTable.offsets = offsets;
                g_state.abiTable.source = sha;
            }
            AC_LOG_INFO(kModule, "ABI table: %zu field offset(s) published with the patterns.",
                        offsets.size());
        }

        bool ParsePatternFile(const std::string& path, const std::string& moduleName,
            toml::table& outTable, PatternIndex& outIndex) {
            std::ifstream file(path);
            if (!file.is_open()) return false;
            try {
                outTable = toml::parse(file);
                const bool indexed = BuildIndex(outTable, moduleName, outIndex);
                CollectAbiTable(outTable, moduleName,
                                std::filesystem::path(path).stem().string());
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

    }  // namespace

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

    namespace {

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
            const auto settings = Settings::Snapshot();
            if (!settings->patternMirror.empty()) return "mirror";
            const downloader::Kind kind = downloader::KindFromName(moduleName);
            for (const downloader::Source& src : downloader::DefaultSources()) {
                if (downloader::IsOstSource(src) && !settings->patternUseOstSource) continue;
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

    }  // namespace

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

}  // namespace ac::pattern::detail
