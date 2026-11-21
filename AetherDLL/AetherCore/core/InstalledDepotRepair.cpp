#include "pch.h"
#include "core/InstalledDepotRepair.h"

#include <algorithm>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <unordered_map>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "network/ManifestIdentity.h"
#include "scripting/LuaData.h"
#include "utils/SteamLibraryPaths.h"
#include "utils/Strings.h"
#include "utils/WinFile.h"

namespace ac::appmanifest {
namespace {

constexpr const char* kModule = "InstalledDepots";
constexpr const char* kManifestSuffix = ".manifest";

struct IndexedFile {
    std::string gid;
    std::string path;
    std::uint64_t modified = 0; // file write time; higher is newer
};

// depotId -> the manifests that depot has on disk. Built from names and file
// metadata only: no manifest is parsed until a depot actually needs a repair.
using DepotIndex = std::unordered_map<std::uint32_t, std::vector<IndexedFile>>;

bool ReadFileBytes(const std::string& path, std::string& out) {
    std::ifstream input(winfile::ToPath(path), std::ios::binary);
    if (!input.is_open())
        return false;
    out.assign(std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>());
    return input.good() || input.eof();
}

// `<depot>_<gid>.manifest` -> depot + gid. False for anything else.
bool ParseManifestName(const std::string& name, std::uint32_t& depotId, std::string& gid) {
    const std::size_t suffixLength = std::char_traits<char>::length(kManifestSuffix);
    if (name.size() <= suffixLength + 2)
        return false;
    if (name.compare(name.size() - suffixLength, suffixLength, kManifestSuffix) != 0)
        return false;
    const std::string stem = name.substr(0, name.size() - suffixLength);
    const std::size_t separator = stem.find('_');
    if (separator == std::string::npos || separator == 0 || separator + 1 >= stem.size())
        return false;
    depotId = ParseDepotId(std::string_view(stem).substr(0, separator));
    const std::string_view gidText = std::string_view(stem).substr(separator + 1);
    if (depotId == 0 || ParseGid(gidText) == 0)
        return false;
    gid.assign(gidText);
    return true;
}

// The two cache folders Steam reads manifests from. The secondary one
// (`config\\depotcache`) holds manifests Steam was handed without a download.
std::vector<std::string> DepotCacheDirs() {
    std::vector<std::string> dirs;
    if (g_state.steamInstallPath.empty())
        return dirs;
    const std::filesystem::path root(winfile::ToPath(g_state.steamInstallPath));
    dirs.push_back(ac::strings::Narrow((root / "depotcache").wstring()));
    dirs.push_back(ac::strings::Narrow((root / "config" / "depotcache").wstring()));
    return dirs;
}

DepotIndex BuildIndex(const std::vector<std::string>& directories) {
    DepotIndex index;
    for (const std::string& directory : directories) {
        std::error_code ec;
        std::filesystem::directory_iterator it(winfile::ToPath(directory), ec);
        const std::filesystem::directory_iterator end;
        if (ec)
            continue;
        for (; it != end; it.increment(ec)) {
            if (ec)
                break;
            if (!it->is_regular_file(ec) || ec)
                continue;
            const std::string name = ac::strings::Narrow(it->path().filename().wstring());
            std::uint32_t depotId = 0;
            std::string gid;
            if (!ParseManifestName(name, depotId, gid))
                continue;
            if (it->file_size(ec) == 0 || ec)
                continue;

            std::vector<IndexedFile>& files = index[depotId];
            const bool known =
                std::any_of(files.begin(), files.end(), [&gid](const IndexedFile& file) { return file.gid == gid; });
            if (known)
                continue;

            IndexedFile file;
            file.gid = gid;
            file.path = ac::strings::Narrow(it->path().wstring());
            const std::filesystem::file_time_type stamp = std::filesystem::last_write_time(it->path(), ec);
            if (!ec) {
                const auto count = stamp.time_since_epoch().count();
                file.modified = count > 0 ? static_cast<std::uint64_t>(count) : 0;
            }
            files.push_back(std::move(file));
        }
    }
    return index;
}

bool IndexHas(const DepotIndex& index, std::uint32_t depotId, const std::string& gid) {
    const auto found = index.find(depotId);
    if (found == index.end())
        return false;
    return std::any_of(found->second.begin(), found->second.end(),
                       [&gid](const IndexedFile& file) { return file.gid == gid; });
}

// Reads and parses only the manifests of a depot that actually needs a repair.
std::vector<LocalManifest> Candidates(const DepotIndex& index, std::uint32_t depotId) {
    std::vector<LocalManifest> candidates;
    const auto found = index.find(depotId);
    if (found == index.end())
        return candidates;
    candidates.reserve(found->second.size());
    for (const IndexedFile& file : found->second) {
        LocalManifest candidate;
        candidate.gid = file.gid;
        candidate.modified = file.modified;
        std::string bytes;
        if (ReadFileBytes(file.path, bytes)) {
            std::uint64_t size = 0;
            if (manifestidentity::ManifestContentSize(bytes, size)) {
                candidate.hasContentSize = true;
                candidate.contentSize = size;
            }
        }
        candidates.push_back(std::move(candidate));
    }
    return candidates;
}

} // namespace

RepairStats ApplyInstalledDepotRepairs() {
    RepairStats stats;
    if (g_state.steamInstallPath.empty()) {
        AC_LOG_DEBUG(kModule, "Repair pass skipped: the Steam path is unknown.");
        return stats;
    }
    if (luadata::LibraryAppIds().empty()) {
        AC_LOG_DEBUG(kModule, "Repair pass skipped: no managed app.");
        return stats;
    }
    const DepotIndex index = BuildIndex(DepotCacheDirs());
    const auto isBacked = [&index](std::uint32_t depotId, const std::string& gid) {
        return IndexHas(index, depotId, gid);
    };
    const auto candidates = [&index](std::uint32_t depotId) { return Candidates(index, depotId); };

    for (const std::string& appDir : steamlibrary::SteamAppsDirs(g_state.steamInstallPath)) {
        std::error_code ec;
        std::filesystem::directory_iterator it(winfile::ToPath(appDir), ec);
        const std::filesystem::directory_iterator end;
        if (ec) {
            ++stats.errors;
            AC_LOG_DEBUG(kModule, "Cannot enumerate '%s' (%d).", appDir.c_str(), ec.value());
            continue;
        }
        for (; it != end; it.increment(ec)) {
            if (ec) {
                ++stats.errors;
                break;
            }
            if (!it->is_regular_file(ec) || ec)
                continue;
            const std::string name = ac::strings::Narrow(it->path().filename().wstring());
            const std::uint32_t appId = AppIdFromManifestName(name);
            if (appId == 0 || !luadata::HasDepot(appId))
                continue;

            ++stats.scanned;
            const std::string path = ac::strings::Narrow(it->path().wstring());
            std::string content;
            if (!ReadFileBytes(path, content)) {
                ++stats.errors;
                AC_LOG_DEBUG(kModule, "Cannot read appmanifest_%u.acf; leaving it untouched.", appId);
                continue;
            }

            const RepairPlan plan = PlanInstalledDepotRepairs(content, isBacked, candidates);
            if (plan.repairs.empty())
                continue;
            // Steam rewrites the ACF itself at the start and at the end of an
            // update, and our write replaces the whole file: skipping an app
            // Steam is already updating avoids reverting what it just wrote.
            // The entry stays broken only until the next start, when the
            // update has settled.
            if ((ReadStateFlags(content) & kAppStateUpdatingMask) != 0) {
                AC_LOG_DEBUG(kModule, "App %u is updating; its InstalledDepots entry is left for the next start.",
                             appId);
                continue;
            }
            if (!winfile::WriteTextAtomically(path, plan.content)) {
                ++stats.errors;
                AC_LOG_WARN(
                    kModule,
                    "Cannot rewrite appmanifest_%u.acf (locked or read-only); retrying at the next Steam start.",
                    appId);
                continue;
            }
            for (const Repair& repair : plan.repairs) {
                ++stats.repaired;
                AC_LOG_INFO(kModule,
                            "App %u: depot %u installed manifest %s -> %s (the declared GID has no local manifest).",
                            appId, repair.depotId, repair.declaredGid.c_str(), repair.restoredGid.c_str());
            }
        }
    }

    if (stats.errors != 0)
        AC_LOG_WARN(kModule, "Repair pass: scanned=%zu repaired=%zu errors=%zu.", stats.scanned, stats.repaired,
                    stats.errors);
    else if (stats.repaired != 0)
        AC_LOG_INFO(kModule, "Repair pass: scanned=%zu repaired=%zu.", stats.scanned, stats.repaired);
    else
        AC_LOG_DEBUG(kModule, "Repair pass: scanned=%zu, every installed depot manifest is on disk.", stats.scanned);
    return stats;
}

void ApplyInstalledDepotRepairsAtStartup() {
    if (!workers::Submit([] { ApplyInstalledDepotRepairs(); }))
        AC_LOG_DEBUG(kModule, "Startup repair pass skipped: worker queue unavailable.");
}

} // namespace ac::appmanifest
