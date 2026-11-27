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
#include "core/AppManifestOwner.h"
#include "core/FamilyClassification.h"
#include "core/Logger.h"
#include "core/OwnerRecordPolicy.h"
#include "core/Settings.h"
#include "core/Workers.h"
#include "credentials/SteamId.h"
#include "family/FamilyLenderPolicy.h"
#include "hooks/license/LicenseManager.h"
#include "scripting/LuaData.h"
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

bool IsLastOwnerKey(std::string_view key) { return ac::paths::LowerAscii(key) == "lastowner"; }

// localconfig.vdf's apptickets section stores one hex-encoded AppOwnershipTicket
// per installed app; its single granting package lives in licenses[0] at byte 44.
void ReadAppTicketPackages(const std::string& localConfig, std::unordered_map<steam::AppId, std::uint32_t>& appPackages,
                           std::size_t& skippedTickets) {
    skippedTickets = 0;
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
            value.size() >= policy::kMinAppTicketHexChars) {
            std::uint64_t appId = 0;
            if (ParseDecimal(key, appId) && appId > 0 && appId <= 0xFFFFFFFF) {
                const auto typedAppId = static_cast<steam::AppId>(appId);
                if (const auto packageId = policy::ExtractTicketPrimaryPackageId(typedAppId, value)) {
                    appPackages[typedAppId] = *packageId;
                } else if (value.size() % 2 == 0 &&
                           value.compare(policy::kTicketSteamIdUpperHexChar, 8, "01001001") == 0) {
                    ++skippedTickets;
                }
            }
        }

        if (lineEnd == localConfig.size())
            break;
        lineStart = lineEnd + 1;
    }
}

// Every installed app manifest can carry LastOwner. For a family-shared app
// that is the real lender; the package id ties other apps in the same family
// package to the same account when their own manifest has no LastOwner. Stale
// Aether banner stamps recorded in owner_state.txt are filtered via
// ResolveEffectiveManifestOwner so a banner switch never poses as a Valve share.
void ReadLastOwners(const std::string& steamRoot, std::uint32_t activeAccount,
                    const std::unordered_map<steam::AppId, std::uint32_t>& appPackages,
                    const std::unordered_map<steam::AppId, ac::appmanifest::OwnerRecord>& ownerRecords,
                    const std::shared_ptr<const Settings>& settings,
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
            const auto app = static_cast<steam::AppId>(appId);

            const std::string manifest = ReadTextFile(entry.path().string());
            if (manifest.empty())
                continue;

            const auto recIt = ownerRecords.find(app);
            std::uint32_t rawOwner = 0;
            bool sawLastOwner = false;

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
                    sawLastOwner = true;
                    std::uint64_t steamId = 0;
                    if (ParseDecimal(value, steamId) && steamId != 0)
                        rawOwner = static_cast<std::uint32_t>(steamId);
                    break;
                }

                if (lineEnd == manifest.size())
                    break;
                lineStart = lineEnd + 1;
            }

            if (!sawLastOwner && recIt == ownerRecords.end())
                continue;

            ac::appmanifest::ManifestOwnerEvidence evidence;
            evidence.manifestLastOwner = rawOwner;
            if (recIt != ownerRecords.end()) {
                evidence.recorded = true;
                evidence.record = recIt->second;
            }
            evidence.activeAccountId = activeAccount;
            evidence.ownBannerNow = rawOwner != 0 && settings &&
                                    ac::familyclass::IsOwnBannerStamp(
                                        app, rawOwner, settings->libraryOwnerBanner, settings->libraryOwnerBannerApps,
                                        settings->libraryOwnerBannerSteamId, activeAccount);

            const std::uint32_t owner = ac::appmanifest::ResolveEffectiveManifestOwner(evidence);
            if (owner != 0) {
                directOwners[app] = owner;
                const auto package = appPackages.find(app);
                if (package != appPackages.end())
                    packageOwners.emplace(package->second, owner);
            } else if (rawOwner != 0 && rawOwner != activeAccount) {
                AC_LOG_DEBUG(kModule, "App %u: ignored stale Aether LastOwner=%u (recorded=%llu).", app, rawOwner,
                             static_cast<unsigned long long>(evidence.record.written));
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
    std::size_t skippedTickets = 0;
    ReadAppTicketPackages(localConfig, appPackages, skippedTickets);

    const auto ownerRecords = ac::appmanifest::LoadOwnerRecordsSnapshot();
    const auto settings = Settings::Snapshot();

    std::unordered_map<steam::AppId, std::uint32_t> directOwners;
    std::unordered_map<std::uint32_t, std::uint32_t> packageOwners;
    std::size_t manifestCount = 0;
    ReadLastOwners(g_state.steamInstallPath, activeAccount, appPackages, ownerRecords, settings, directOwners,
                   packageOwners, manifestCount);

    const auto resolved = policy::ResolveLenders(appPackages, directOwners, packageOwners);
    std::unordered_map<steam::AppId, std::uint32_t> lenders;
    lenders.reserve(resolved.size());
    for (const auto& [app, entry] : resolved)
        lenders[app] = entry.lenderAccountId;

    bool changed = false;
    std::size_t lenderCount = 0;
    std::unordered_map<steam::AppId, std::uint32_t> previousLenders;
    std::unordered_map<steam::AppId, std::uint32_t> lenderSnapshot;
    {
        std::unique_lock lock(s_mutex);
        changed = lenders != s_lenders;
        if (changed)
            previousLenders = std::move(s_lenders);
        s_lenders = std::move(lenders);
        s_ready = true;
        lenderCount = s_lenders.size();
        lenderSnapshot = s_lenders;
    }

    if (changed) {
        for (const auto& [app, entry] : resolved) {
            const auto prev = previousLenders.find(app);
            if (prev == previousLenders.end() || prev->second != entry.lenderAccountId) {
                const char* sourceName =
                    entry.source == policy::LenderSource::DirectManifest ? "manifest_lastowner" : "shared_package";
                AC_LOG_INFO(kModule, "Lender mapped: app=%u -> lender=%u (source=%s, pkg=%u, prev=%u).", app,
                            entry.lenderAccountId, sourceName, entry.packageId,
                            prev != previousLenders.end() ? prev->second : 0u);
            }
            const bool isValveShare =
                !settings
                    ? ac::familyclass::IsValveFamilyLicense(app, entry.lenderAccountId, activeAccount, false, {}, 0)
                    : ac::familyclass::IsValveFamilyLicense(
                          app, entry.lenderAccountId, activeAccount, settings->libraryOwnerBanner,
                          settings->libraryOwnerBannerApps, settings->libraryOwnerBannerSteamId);
            if (isValveShare && ac::luadata::IsConfigured(app))
                ac::hooks::LicenseManager::RequestRemoveAppFromPackage0(app);
        }
        for (const auto& [app, prevLender] : previousLenders) {
            if (resolved.find(app) == resolved.end()) {
                AC_LOG_INFO(kModule, "Lender unmapped: app=%u (was_lender=%u).", app, prevLender);
            }
        }
    }

    bool reconciled = false;
    for (const steam::AppId app : ac::luadata::MapDerivedFamilyAppIds()) {
        const auto it = lenderSnapshot.find(app);
        const std::uint32_t currentLender = it != lenderSnapshot.end() ? it->second : 0;
        const bool ownBannerNow =
            settings && ac::familyclass::IsOwnBannerStamp(app, currentLender, settings->libraryOwnerBanner,
                                                          settings->libraryOwnerBannerApps,
                                                          settings->libraryOwnerBannerSteamId, activeAccount);
        if (ac::familyclass::ShouldClearMapFamilyShare(true, !ac::luadata::IsRealFamilyShare(app), currentLender,
                                                       ownBannerNow) &&
            ac::luadata::ClearMapFamilyShare(app)) {
            AC_LOG_INFO(kModule,
                        "App %u: cleared map-derived family share after lender refresh (lender=%u, own_banner=%d); "
                        "re-queuing package-0 license.",
                        app, currentLender, ownBannerNow ? 1 : 0);
            ac::hooks::LicenseManager::RequestAppInPackage0(app);
            reconciled = true;
        }
    }

    if (changed || reconciled)
        ac::hooks::LicenseManager::RequestLicenseRefresh();

    AC_LOG_INFO(kModule, "Family lenders: %zu app(s), %zu ticket(s), %zu skipped, %zu manifest(s).", lenderCount,
                appPackages.size(), skippedTickets, manifestCount);
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
