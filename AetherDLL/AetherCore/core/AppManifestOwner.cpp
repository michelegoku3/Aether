#include "pch.h"
#include "core/AppManifestOwner.h"

#include <atomic>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <mutex>
#include <string>
#include <system_error>

#include "core/AetherCoreState.h"
#include "core/AppManifestText.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "scripting/LuaData.h"
#include "utils/SteamLibraryPaths.h"
#include "utils/Strings.h"
#include "utils/WinFile.h"

namespace ac::appmanifest {
namespace {

constexpr const char* kModule = "OwnerStamp";

constexpr const char* kManifestPrefix = "appmanifest_";
constexpr const char* kManifestSuffix = ".acf";

std::mutex s_passMutex; // one pass at a time (boot job vs post-login job)
std::atomic<bool> s_pending{false};

bool ReadTextFile(const std::string& path, std::string& out) {
    std::ifstream input(winfile::ToPath(path), std::ios::binary);
    if (!input.is_open())
        return false;
    out.assign(std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>());
    return input.good() || input.eof();
}

// Atomic replace that preserves a read-only attribute (version pinning marks
// some appmanifests read-only; Steam and AetherDesk both write them).
bool WriteTextAtomically(const std::string& path, const std::string& content) {
    const std::string temp = path + ".aether-owner.tmp";
    {
        std::ofstream output(winfile::ToPath(temp), std::ios::binary | std::ios::trunc);
        if (!output.is_open())
            return false;
        output.write(content.data(), static_cast<std::streamsize>(content.size()));
        output.flush();
        if (!output.good()) {
            output.close();
            winfile::Delete(temp);
            return false;
        }
    }

    const std::wstring wide = winfile::WidenPath(path);
    const DWORD attributes = GetFileAttributesW(wide.c_str());
    const bool readOnly = attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_READONLY) != 0;
    if (readOnly)
        SetFileAttributesW(wide.c_str(), attributes & ~FILE_ATTRIBUTE_READONLY);

    const bool replaced = winfile::MoveReplace(temp, path);

    if (readOnly && attributes != INVALID_FILE_ATTRIBUTES)
        SetFileAttributesW(wide.c_str(), attributes);
    if (!replaced)
        winfile::Delete(temp);
    return replaced;
}

// Parses `appmanifest_<appId>.acf`; returns 0 for anything else.
steam::AppId AppIdFromManifestName(const std::string& name) {
    const std::size_t prefixLength = std::char_traits<char>::length(kManifestPrefix);
    const std::size_t suffixLength = std::char_traits<char>::length(kManifestSuffix);
    if (name.size() <= prefixLength + suffixLength)
        return 0;
    if (name.compare(0, prefixLength, kManifestPrefix) != 0)
        return 0;
    if (name.compare(name.size() - suffixLength, suffixLength, kManifestSuffix) != 0)
        return 0;

    const std::string stem = name.substr(prefixLength, name.size() - prefixLength - suffixLength);
    char* end = nullptr;
    const unsigned long value = std::strtoul(stem.c_str(), &end, 10);
    if (!end || *end != '\0' || value == 0 || value > 0xFFFFFFFFul)
        return 0;
    return static_cast<steam::AppId>(value);
}

void ApplyToManifest(const std::string& path, steam::AppId appId, StripStats& stats) {
    std::string content;
    if (!ReadTextFile(path, content)) {
        ++stats.errors;
        AC_LOG_DEBUG(kModule, "Cannot read appmanifest_%u.acf; leaving it untouched.", appId);
        return;
    }
    if (!RemoveLastOwner(content)) {
        ++stats.unchanged;
        return;
    }
    if (!WriteTextAtomically(path, content)) {
        ++stats.errors;
        AC_LOG_WARN(kModule, "Cannot drop LastOwner from appmanifest_%u.acf (locked or read-only).", appId);
        return;
    }
    ++stats.removed;
    AC_LOG_INFO(kModule, "App %u: LastOwner dropped.", appId);
}

} // namespace

StripStats StripInstalledAppOwners() {
    StripStats stats;

    std::lock_guard<std::mutex> passLock(s_passMutex);
    for (const std::string& dir : steamlibrary::SteamAppsDirs(g_state.steamInstallPath)) {
        std::error_code ec;
        std::filesystem::directory_iterator it(winfile::ToPath(dir), ec);
        if (ec) {
            ++stats.errors;
            AC_LOG_DEBUG(kModule, "Cannot enumerate '%s' (%d).", dir.c_str(), ec.value());
            continue;
        }
        const std::filesystem::directory_iterator end;
        for (; it != end; it.increment(ec)) {
            if (ec) {
                ++stats.errors;
                break;
            }
            if (!it->is_regular_file(ec) || ec)
                continue;
            const std::string name = ac::strings::Narrow(it->path().filename().wstring());
            const steam::AppId appId = AppIdFromManifestName(name);
            if (appId == 0)
                continue;
            // Only apps Aether actually spoofs: a genuinely owned or
            // family-shared game must keep Steam's real owner record.
            if (!luadata::HasDepot(appId))
                continue;

            ++stats.scanned;
            const std::string path = ac::strings::Narrow(it->path().wstring());
            ApplyToManifest(path, appId, stats);
        }
    }

    if (stats.removed != 0 || stats.errors != 0) {
        AC_LOG_INFO(kModule, "Owner strip: scanned=%zu removed=%zu unchanged=%zu errors=%zu.", stats.scanned,
                    stats.removed, stats.unchanged, stats.errors);
    } else {
        AC_LOG_DEBUG(kModule, "Owner strip: scanned=%zu, nothing to change.", stats.scanned);
    }
    return stats;
}

void RequestStrip() {
    bool expected = false;
    if (!s_pending.compare_exchange_strong(expected, true, std::memory_order_acq_rel))
        return;
    if (!workers::Submit([] {
            s_pending.store(false, std::memory_order_release);
            StripInstalledAppOwners();
        })) {
        s_pending.store(false, std::memory_order_release);
    }
}

void StripAtStartup() {
    if (!workers::Submit([] { StripInstalledAppOwners(); })) {
        AC_LOG_DEBUG(kModule, "Startup owner strip skipped: worker queue unavailable.");
    }
}

} // namespace ac::appmanifest
