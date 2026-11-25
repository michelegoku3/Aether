#include "pch.h"
#include "core/Logger.h"
#include "utils/Hasher.h"
#include "utils/PatternCache.h"
#include "utils/PatternDownloader.h"
#include "utils/PatternSidecar.h"
#include "utils/PatternState.h"
#include "utils/WinFile.h"
#include <filesystem>
#include <fstream>
#include <toml++/toml.hpp>
#include <unordered_map>

namespace ac::pattern::detail {
namespace {
constexpr const char* kModule = "PatternEngine";
// Bound the optional upgrade probe; a valid cache remains usable.
constexpr int kUpgradeProbeTimeoutSec = 5;
using PatternEntry = AetherCoreState::PatternEntry;

void SetPatternStatus(const std::string& module, bool found, const std::string& source, std::uint64_t version) {
    std::lock_guard lock(g_state.statusMetadataMutex);
    if (module == "steamclient") {
        g_state.steamclientTomlFound = found;
        g_state.steamclientPatternSource = source;
        g_state.steamclientPatternVersion = version;
    } else if (module == "steamui") {
        g_state.steamuiTomlFound = found;
        g_state.steamuiPatternSource = source;
        g_state.steamuiPatternVersion = version;
    }
}

bool BuildIndex(const toml::table& table, const std::string& moduleName, PatternIndex& outIndex) {
    outIndex.clear();
    for (const auto& [_, val] : table) {
        const auto* sub = val.as_table();
        if (!sub)
            continue;

        std::string name = sub->at_path("name").value_or(std::string{});
        std::string rva = sub->at_path("rva").value_or(std::string{});
        std::string sig = sub->at_path("sig").value_or(std::string{});
        if (name.empty() || rva.empty())
            continue;

        auto [it, inserted] = outIndex.emplace(name, PatternEntry{rva, sig});
        if (!inserted) {
            AC_LOG_WARN(kModule, "Duplicate pattern name '%s' in %s; keeping first entry.", name.c_str(),
                        moduleName.c_str());
            std::string tag;
            tag.reserve(moduleName.size() + name.size() + 1);
            tag += moduleName;
            tag += ':';
            tag += name;
            diag::Record("pattern_duplicate", tag);
        }
    }
    return !outIndex.empty();
}

// Reads the optional `[abi.<Struct>.<field>]` sections a table may carry.
// Absence is not fatal: a build without an ABI section simply falls back
// to the probe. Both shipped tables can publish offsets, so they are stored
// per module: a steamui table must not replace the steamclient one.
void CollectAbiTable(const toml::table& table, const std::string& moduleName, const std::string& sha) {
    const bool steamclient = moduleName == "steamclient";
    const bool steamui = moduleName == "steamui";
    if (!steamclient && !steamui)
        return;
    const auto* abi = table["abi"].as_table();
    if (!abi)
        return;

    std::unordered_map<std::string, std::uint32_t> offsets;
    for (const auto& [structKey, structNode] : *abi) {
        const auto* fields = structNode.as_table();
        if (!fields)
            continue;
        for (const auto& [fieldKey, fieldNode] : *fields) {
            const auto* entry = fieldNode.as_table();
            if (!entry)
                continue;
            const auto value = (*entry)["offset"].value<std::int64_t>();
            if (!value || *value < 0 || *value > 0xFFFF)
                continue;
            offsets.emplace(std::string(structKey.str()) + "." + std::string(fieldKey.str()),
                            static_cast<std::uint32_t>(*value));
        }
    }
    if (offsets.empty())
        return;

    const std::size_t count = offsets.size();
    {
        std::lock_guard lock(g_state.abiTable.mutex);
        if (steamclient) {
            g_state.abiTable.offsets = std::move(offsets);
            g_state.abiTable.source = sha;
        } else {
            g_state.abiTable.uiOffsets = std::move(offsets);
            g_state.abiTable.uiSource = sha;
        }
    }
    AC_LOG_INFO(kModule, "ABI table: %zu field offset(s) published with the %s patterns.", count, moduleName.c_str());
}

bool ParsePatternFile(const std::string& path, const std::string& moduleName, toml::table& outTable,
                      PatternIndex& outIndex, bool collectAbi = true) {
    std::ifstream file(winfile::ToPath(path));
    if (!file.is_open())
        return false;
    try {
        outTable = toml::parse(file);
        const bool indexed = BuildIndex(outTable, moduleName, outIndex);
        if (collectAbi)
            CollectAbiTable(outTable, moduleName, std::filesystem::path(path).stem().string());
        AC_LOG_INFO(kModule, "Loaded %zu TOML section(s), indexed %zu pattern name(s) for %s from %s.", outTable.size(),
                    outIndex.size(), moduleName.c_str(), path.c_str());
        return indexed;
    } catch (const toml::parse_error& err) {
        AC_LOG_ERROR(kModule, "TOML parse error for %s cache %s: %s", moduleName.c_str(), path.c_str(), err.what());
        return false;
    }
}

} // namespace

void SweepTempFiles() {
    namespace fs = std::filesystem;
    std::error_code ec;
    if (!fs::is_directory(winfile::ToPath(g_state.patternDir), ec))
        return;
    for (const auto& entry : fs::directory_iterator(winfile::ToPath(g_state.patternDir), ec)) {
        if (ec)
            break;
        if (!entry.is_regular_file(ec))
            continue;
        const std::string name = entry.path().filename().string();
        if (name.size() >= 4 && name.compare(name.size() - 4, 4, ".tmp") == 0) {
            fs::remove(entry.path(), ec);
        }
    }
}

namespace {

// Provenance sidecar.
// Every cache TOML is paired with a "<sha>.toml.src" sidecar holding the exact
// source label that produced the file (e.g. "migo:raw") and the generation of
// the published table it holds. PatternSidecar owns the format; this file owns
// what the pair means.

// The source a fresh download would be expected to carry for this module
// right now: the configured user mirror if any, else the first source in
// the registry that carries the kind. Empty when nothing can serve it.
std::string BestExpectedSource(const std::string& moduleName) {
    const auto settings = Settings::Snapshot();
    if (!settings->patternMirror.empty())
        return "mirror";
    const downloader::Kind kind = downloader::KindFromName(moduleName);
    for (const downloader::Source& src : downloader::DefaultSources()) {
        if (src.LocFor(kind) != nullptr)
            return std::string(src.id);
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
    if (expected.empty())
        return true; // no better source exists
    if (provenance == "mirror")
        return expected == "mirror";
    const std::size_t colon = provenance.find(':');
    const std::string source = (colon == std::string::npos) ? provenance : provenance.substr(0, colon);
    return source == expected;
}

// Number of entries a TOML file would index. Used to refuse an "upgrade"
// that would shrink the table we already have.
std::size_t CountPatternEntries(const std::string& path) {
    try {
        std::ifstream file(winfile::ToPath(path));
        if (!file.is_open())
            return 0;
        const toml::table parsed = toml::parse(file);
        std::size_t count = 0;
        for (const auto& [_, val] : parsed) {
            const auto* sub = val.as_table();
            if (!sub)
                continue;
            if (sub->at_path("name").value_or(std::string{}).empty())
                continue;
            if (sub->at_path("rva").value_or(std::string{}).empty())
                continue;
            ++count;
        }
        return count;
    } catch (...) {
        return 0;
    }
}

} // namespace

// Loads the pattern table for one module. A corrupt cache is not fatal:
// it is quarantined and the downloader gets one chance to refresh the
// TOML.
//
// Boot path is strictly local-first: a usable cache is served immediately with
// zero network round-trips so hook installation is never delayed by the
// remote generation index. RefreshCachedModule handles background updates
// after hooks are installed.
bool LoadModule(const std::string& moduleName, const std::string& dllPath, std::string& outSha,
                PatternIndex& outIndex) {
    outIndex.clear();
    outSha = hasher::ComputeFileSha256(dllPath);
    if (outSha.empty()) {
        // Without a SHA we cannot name the per-build cache file or fetch the
        // correct table: address resolution is unavailable for this module.
        AC_LOG_ERROR(kModule, "Could not hash %s; no patterns available.", dllPath.c_str());
        SetPatternStatus(moduleName, false, "hash-failed", 0);
        diag::Record("pattern_missing", moduleName);
        return false;
    }
    AC_LOG_INFO(kModule, "%s SHA-256: %s", moduleName.c_str(), outSha.c_str());

    // toml::table is temporary -- BuildIndex extracts everything into
    // PatternIndex, which is the only structure used at runtime.
    toml::table table;

    const std::string tomlPath = g_state.patternDir + "\\" + outSha + ".toml";
    const std::string srcPath = tomlPath + ".src";

    // Local cache: parse it when present (a failure is quarantined and
    // triggers the download path below).
    if (winfile::Exists(tomlPath)) {
        const bool cacheUsable = ParsePatternFile(tomlPath, moduleName, table, outIndex);
        if (cacheUsable) {
            const sidecar::Record applied = sidecar::Read(srcPath);
            const std::string source = applied.source.empty() ? "cache" : applied.source + " (cache)";
            SetPatternStatus(moduleName, true, source, applied.version);
            diag::Record("pattern", moduleName + ":" + source);
            return true;
        }
        const std::string badPath = tomlPath + ".bad";
        winfile::MoveReplace(tomlPath, badPath);
        winfile::Delete(srcPath); // stale provenance dies with the file
        AC_LOG_WARN(kModule, "Quarantined corrupt %s pattern cache as %s.", moduleName.c_str(), badPath.c_str());
        diag::Record("pattern_cache_bad", moduleName);
    } else {
        AC_LOG_INFO(kModule, "No cached pattern for %s; downloading...", moduleName.c_str());
    }

    std::string downloadSource;
    if (!downloader::Download(moduleName, outSha, tomlPath, &downloadSource)) {
        AC_LOG_WARN(kModule, "Download failed for %s; no patterns available.", moduleName.c_str());
        SetPatternStatus(moduleName, false, "missing", 0);
        diag::Record("pattern_missing", moduleName);
        return false;
    }

    if (ParsePatternFile(tomlPath, moduleName, table, outIndex)) {
        const std::string source = downloadSource.empty() ? "download" : downloadSource;
        // Generation 0 until the background index check confirms the published
        // generation, avoiding an extra blocking GET before hook installation.
        if (!sidecar::Write(srcPath, source, 0)) {
            AC_LOG_WARN(kModule, "Could not record the %s pattern source; the next launch may refresh again.",
                        moduleName.c_str());
        }
        SetPatternStatus(moduleName, true, source, 0);
        diag::Record("pattern", moduleName + ":" + source);
        return true;
    }

    winfile::Delete(tomlPath);
    winfile::Delete(srcPath);
    SetPatternStatus(moduleName, false, "invalid", 0);
    diag::Record("pattern_invalid", moduleName);
    AC_LOG_ERROR(kModule, "Downloaded pattern for %s is invalid; cache removed.", moduleName.c_str());
    return false;
}

bool RefreshCachedModule(const std::string& moduleName, const std::string& dllPath) {
    if (dllPath.empty())
        return false;
    const std::string sha = hasher::ComputeFileSha256(dllPath);
    if (sha.empty())
        return false;

    const std::string tomlPath = g_state.patternDir + "\\" + sha + ".toml";
    const std::string srcPath = tomlPath + ".src";
    if (!winfile::Exists(tomlPath))
        return false;

    // Parse into locals without touching ABI state: hooks are already armed.
    toml::table currentTable;
    PatternIndex currentIndex;
    if (!ParsePatternFile(tomlPath, moduleName, currentTable, currentIndex, false))
        return false;

    const std::string expectedBest = BestExpectedSource(moduleName);
    const std::uint64_t remote = RemoteVersion(moduleName, sha);
    sidecar::Record applied = sidecar::Read(srcPath);
    const RefreshPlan plan = PlanRefresh(true, remote, applied.version, ProvenanceIsBest(applied.source, expectedBest));
    if (!plan.needed)
        return false;

    if (plan.versioned) {
        AC_LOG_INFO(kModule, "Cached %s patterns are generation %llu, the index says %llu; refreshing.",
                    moduleName.c_str(), static_cast<unsigned long long>(applied.version),
                    static_cast<unsigned long long>(remote));
    } else {
        AC_LOG_INFO(kModule,
                    "Cached %s patterns (provenance '%s') are not from the "
                    "preferred source ('%s'); checking upstream.",
                    moduleName.c_str(), applied.source.empty() ? "<unknown>" : applied.source.c_str(),
                    expectedBest.c_str());
    }

    const std::string candidatePath = tomlPath + ".tmp";
    std::string fetched;
    if (!downloader::Download(moduleName, sha, candidatePath, &fetched, kUpgradeProbeTimeoutSec)) {
        AC_LOG_INFO(kModule, "No upstream source can serve %s right now; keeping the cached table.",
                    moduleName.c_str());
        return false;
    }
    if (!AdoptDownload(plan.versioned, applied.version, winfile::FilesEqual(candidatePath, tomlPath))) {
        winfile::Delete(candidatePath);
        AC_LOG_WARN(kModule, "Upstream '%s' re-served the %s table already cached; retrying next launch.",
                    fetched.c_str(), moduleName.c_str());
        return false;
    }
    if (!plan.versioned && CountPatternEntries(candidatePath) < currentIndex.size()) {
        winfile::Delete(candidatePath);
        AC_LOG_INFO(kModule, "Upstream '%s' candidate for %s is smaller than the cached table; keeping the cache.",
                    fetched.c_str(), moduleName.c_str());
        return false;
    }

    toml::table fresh;
    PatternIndex freshIndex;
    if (!ParsePatternFile(candidatePath, moduleName, fresh, freshIndex, false)) {
        winfile::Delete(candidatePath);
        AC_LOG_WARN(kModule, "Upstream %s candidate could not be parsed; keeping the cached table.",
                    moduleName.c_str());
        return false;
    }
    if (!winfile::MoveReplace(candidatePath, tomlPath)) {
        winfile::Delete(candidatePath);
        AC_LOG_WARN(kModule, "Could not adopt upstream %s patterns; keeping the cached table.", moduleName.c_str());
        return false;
    }

    const std::uint64_t adopted = plan.versioned ? remote : applied.version;
    if (sidecar::Write(srcPath, fetched, adopted)) {
        applied.version = adopted;
    } else {
        AC_LOG_WARN(kModule, "Could not record the %s pattern generation; the next launch will refresh.",
                    moduleName.c_str());
    }
    SetPatternStatus(moduleName, true, fetched, applied.version);
    if (plan.versioned) {
        AC_LOG_INFO(kModule, "Refreshed %s patterns to generation %llu from '%s' (%zu entries).", moduleName.c_str(),
                    static_cast<unsigned long long>(applied.version), fetched.c_str(), freshIndex.size());
    } else {
        AC_LOG_INFO(kModule, "Upgraded %s patterns from '%s' (%zu entries).", moduleName.c_str(), fetched.c_str(),
                    freshIndex.size());
    }
    return true;
}

} // namespace ac::pattern::detail
