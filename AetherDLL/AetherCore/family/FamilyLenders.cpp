#include "pch.h"
#include "family/FamilyLenders.h"

#include <algorithm>
#include <atomic>
#include <charconv>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <mutex>
#include <shared_mutex>
#include <string>
#include <string_view>
#include <system_error>
#include <unordered_map>
#include <utility>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "credentials/SteamId.h"
#include "hooks/license/LicenseManager.h"
#include "utils/Paths.h"
#include "utils/VdfText.h"
#include "utils/WinFile.h"

namespace ac::family {
namespace {

constexpr const char* kModule = "FamilyLenders";
constexpr ULONGLONG kRefreshPeriodMs = 5ULL * 60 * 1000;

std::shared_mutex s_mutex;
std::unordered_map<steam::AppId, std::uint32_t> s_lenders;
bool s_ready = false;
std::atomic<bool> s_started{false};
std::atomic<bool> s_refreshRequested{false};

std::string ReadTextFile(const std::string& path) {
    std::ifstream input(winfile::ToPath(path), std::ios::binary);
    if (!input.is_open())
        return {};
    return std::string((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
}

bool IsDigits(std::string_view text) {
    if (text.empty())
        return false;
    return std::all_of(text.begin(), text.end(), [](char ch) { return ch >= '0' && ch <= '9'; });
}

bool ParseDecimal(std::string_view text, std::uint64_t& value) {
    const auto parsed = std::from_chars(text.data(), text.data() + text.size(), value);
    return parsed.ec == std::errc{} && parsed.ptr == text.data() + text.size();
}

bool ParseHex32(std::string_view text, std::uint32_t& value) {
    const auto parsed = std::from_chars(text.data(), text.data() + text.size(), value, 16);
    return parsed.ec == std::errc{} && parsed.ptr == text.data() + text.size();
}

bool IsLastOwnerKey(std::string_view key) { return ac::paths::LowerAscii(key) == "lastowner"; }

// localconfig.vdf's apptickets section stores one app ticket per installed app.
// Its 20th byte is the package the app was issued from; for a family-shared app
// that package belongs to the lender, so it survives the active user's ticket.
void ReadAppTicketPackages(const std::string& localConfig,
                           std::unordered_map<steam::AppId, std::uint32_t>& appPackages) {
    std::size_t lineStart = 0;
    while (lineStart <= localConfig.size()) {
        std::size_t lineEnd = localConfig.find('\n', lineStart);
        if (lineEnd == std::string::npos)
            lineEnd = localConfig.size();
        std::string line = localConfig.substr(lineStart, lineEnd - lineStart);
        if (!line.empty() && line.back() == '\r')
            line.pop_back();

        std::string key;
        std::string value;
        if (ac::vdf::ParseLine(line, key, value) == ac::vdf::LineKind::KeyValue && IsDigits(key) &&
            value.size() >= 48 && value.size() % 2 == 0 && value.compare(24, 8, "01001001") == 0) {
            std::uint64_t appId = 0;
            std::uint32_t packageId = 0;
            if (ParseDecimal(key, appId) && appId <= 0xFFFFFFFF &&
                ParseHex32(std::string_view(value).substr(40, 8), packageId) && packageId != 0) {
                appPackages[static_cast<steam::AppId>(appId)] = packageId;
            }
        }

        if (lineEnd == localConfig.size())
            break;
        lineStart = lineEnd + 1;
    }
}

// Every installed app manifest can carry LastOwner. For a family-shared app
// that is the real lender; the package id ties other apps in the same family
// package to the same account when their own manifest has no LastOwner.
void ReadLastOwners(const std::string& steamRoot, std::uint32_t activeAccount,
                    const std::unordered_map<steam::AppId, std::uint32_t>& appPackages,
                    std::unordered_map<steam::AppId, std::uint32_t>& directOwners,
                    std::unordered_map<std::uint32_t, std::uint32_t>& packageOwners, std::size_t& manifestCount) {
    std::vector<std::string> libraries;
    libraries.push_back(steamRoot + "\\steamapps");

    for (const char* libraryFoldersPath : {"\\steamapps\\libraryfolders.vdf", "\\config\\libraryfolders.vdf"}) {
        const std::string content = ReadTextFile(steamRoot + libraryFoldersPath);
        if (content.empty())
            continue;
        for (const std::string& path : ac::vdf::ExtractQuotedValues(content, "path")) {
            if (!path.empty())
                libraries.push_back(path + "\\steamapps");
        }
        break;
    }

    std::sort(libraries.begin(), libraries.end());
    libraries.erase(std::unique(libraries.begin(), libraries.end()), libraries.end());

    for (const std::string& library : libraries) {
        std::error_code error;
        std::filesystem::directory_iterator iterator(winfile::ToPath(library), error);
        if (error)
            continue;
        for (const std::filesystem::directory_entry& entry : iterator) {
            if (!entry.is_regular_file(error) || error)
                continue;
            const std::string name = entry.path().filename().string();
            if (name.rfind("appmanifest_", 0) != 0 || entry.path().extension() != ".acf")
                continue;

            std::uint64_t appId = 0;
            const std::string idText = name.substr(12, name.size() - 12 - 4);
            if (!IsDigits(idText) || !ParseDecimal(idText, appId) || appId > 0xFFFFFFFF)
                continue;
            ++manifestCount;

            const std::string manifest = ReadTextFile(entry.path().string());
            if (manifest.empty())
                continue;

            std::size_t lineStart = 0;
            while (lineStart <= manifest.size()) {
                std::size_t lineEnd = manifest.find('\n', lineStart);
                if (lineEnd == std::string::npos)
                    lineEnd = manifest.size();
                std::string line = manifest.substr(lineStart, lineEnd - lineStart);
                if (!line.empty() && line.back() == '\r')
                    line.pop_back();

                std::string key;
                std::string value;
                if (ac::vdf::ParseLine(line, key, value) == ac::vdf::LineKind::KeyValue && IsLastOwnerKey(key)) {
                    std::uint64_t steamId = 0;
                    if (ParseDecimal(value, steamId) && steamId != 0) {
                        const auto owner = static_cast<std::uint32_t>(steamId);
                        if (owner != activeAccount) {
                            const auto app = static_cast<steam::AppId>(appId);
                            directOwners[app] = owner;
                            const auto package = appPackages.find(app);
                            if (package != appPackages.end())
                                packageOwners.emplace(package->second, owner);
                        }
                    }
                    break;
                }

                if (lineEnd == manifest.size())
                    break;
                lineStart = lineEnd + 1;
            }
        }
    }
}

bool Refresh() {
    if (g_state.steamInstallPath.empty())
        return false;

    const std::uint64_t activeSteamId = ac::steamid::GetActiveSteamId64();
    if (activeSteamId == 0)
        return false;
    const auto activeAccount = static_cast<std::uint32_t>(activeSteamId);

    const std::string localConfig = ReadTextFile(g_state.steamInstallPath + "\\userdata\\" +
                                                 std::to_string(activeAccount) + "\\config\\localconfig.vdf");
    if (localConfig.empty())
        return false;

    std::unordered_map<steam::AppId, std::uint32_t> appPackages;
    ReadAppTicketPackages(localConfig, appPackages);

    std::unordered_map<steam::AppId, std::uint32_t> directOwners;
    std::unordered_map<std::uint32_t, std::uint32_t> packageOwners;
    std::size_t manifestCount = 0;
    ReadLastOwners(g_state.steamInstallPath, activeAccount, appPackages, directOwners, packageOwners, manifestCount);

    std::unordered_map<steam::AppId, std::uint32_t> lenders;
    for (const auto& [app, package] : appPackages) {
        const auto direct = directOwners.find(app);
        if (direct != directOwners.end()) {
            lenders[app] = direct->second;
            continue;
        }
        const auto owner = packageOwners.find(package);
        if (owner != packageOwners.end())
            lenders[app] = owner->second;
    }
    for (const auto& [app, owner] : directOwners)
        lenders[app] = owner;

    bool changed = false;
    std::size_t lenderCount = 0;
    {
        std::unique_lock lock(s_mutex);
        changed = lenders != s_lenders;
        s_lenders = std::move(lenders);
        s_ready = true;
        lenderCount = s_lenders.size();
    }
    if (changed)
        ac::hooks::LicenseManager::RequestLicenseRefresh();

    AC_LOG_INFO(kModule, "Family lenders: %zu app(s), %zu ticket(s), %zu manifest(s).", lenderCount, appPackages.size(),
                manifestCount);
    return true;
}

void RefreshWorker(std::atomic<bool>& stop) {
    ULONGLONG lastRefresh = 0;
    while (!stop.load(std::memory_order_relaxed)) {
        bool requested = false;
        for (int tick = 0; tick < 100 && !stop.load(std::memory_order_relaxed); ++tick) {
            if (s_refreshRequested.exchange(false)) {
                requested = true;
                break;
            }
            Sleep(10);
        }
        if (stop.load(std::memory_order_relaxed))
            return;

        const bool due = lastRefresh == 0 || GetTickCount64() - lastRefresh >= kRefreshPeriodMs;
        bool ready = false;
        {
            std::shared_lock lock(s_mutex);
            ready = s_ready;
        }
        if (!ready || due || requested) {
            if (Refresh())
                lastRefresh = GetTickCount64();
        }
    }
}

} // namespace

void Init() {
    if (s_started.exchange(true))
        return;
    Refresh();
    if (!ac::workers::StartWorker("family_lenders", RefreshWorker))
        AC_LOG_WARN(kModule, "Family lender worker refused; map stays as loaded.");
}

void Shutdown() {
    s_started.store(false, std::memory_order_relaxed);
    s_refreshRequested.store(false, std::memory_order_relaxed);
    std::unique_lock lock(s_mutex);
    s_lenders.clear();
    s_ready = false;
}

void RequestRefresh() { s_refreshRequested.store(true, std::memory_order_relaxed); }

bool Ready() {
    std::shared_lock lock(s_mutex);
    return s_ready;
}

std::uint32_t LenderFor(steam::AppId appId) {
    std::shared_lock lock(s_mutex);
    const auto it = s_lenders.find(appId);
    return it != s_lenders.end() ? it->second : 0;
}

} // namespace ac::family
