#include "pch.h"
#include "core/AppManifestOwner.h"

#include <atomic>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <mutex>
#include <sstream>
#include <string>
#include <system_error>
#include <unordered_map>
#include <unordered_set>

#include "core/AetherCoreState.h"
#include "core/AppManifestText.h"
#include "core/Logger.h"
#include "core/OwnerRecordPolicy.h"
#include "core/Settings.h"
#include "core/Workers.h"
#include "credentials/SteamId.h"
#include "scripting/LuaData.h"
#include "utils/SteamLibraryPaths.h"
#include "utils/Strings.h"
#include "utils/WinFile.h"

namespace ac::appmanifest {
namespace {

constexpr const char* kModule = "OwnerStamp";

constexpr const char* kManifestPrefix = "appmanifest_";
constexpr const char* kManifestSuffix = ".acf";
constexpr const char* kRecordFileName = "owner_state.txt";

std::mutex s_passMutex; // one pass at a time (boot job vs post-login job)
std::atomic<bool> s_pending{false};

// appId -> what Aether wrote and the value Steam had before. Persisted so a
// later pass can restore Steam's own owner after the app leaves the managed set
// or turns out to be family-shared.
using OwnerRecords = std::unordered_map<steam::AppId, OwnerRecord>;

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

// Records live next to the log/config: <steam>\aethercore\owner_state.txt.
std::string RecordPath() {
    if (g_state.configPath.empty())
        return {};
    const std::filesystem::path config(winfile::ToPath(g_state.configPath));
    return ac::strings::Narrow((config.parent_path() / kRecordFileName).wstring());
}

OwnerRecords LoadRecords() {
    OwnerRecords records;
    const std::string path = RecordPath();
    if (path.empty())
        return records;
    std::string text;
    if (!ReadTextFile(path, text))
        return records;

    // One record per line: "<appId> <written> [<original>]". The two-field form
    // has no captured original (Aether removed the entry); the three-field form
    // carries the pre-Aether value when it was present.
    std::istringstream input(text);
    std::string line;
    while (std::getline(input, line)) {
        std::istringstream fields(line);
        unsigned long long appId = 0;
        unsigned long long written = 0;
        if (!(fields >> appId >> written) || appId == 0 || appId > 0xFFFFFFFFull)
            continue;

        OwnerRecord record;
        record.written = written;
        unsigned long long original = 0;
        if (fields >> original) {
            record.originalKnown = original != 0;
            record.original = original;
        }
        records[static_cast<steam::AppId>(appId)] = record;
    }
    return records;
}

void SaveRecords(const OwnerRecords& records) {
    const std::string path = RecordPath();
    if (path.empty())
        return;
    std::string text;
    for (const auto& [appId, record] : records) {
        text += std::to_string(appId);
        text += '\t';
        text += std::to_string(record.written);
        text += '\t';
        text += std::to_string(record.originalKnown ? record.original : 0);
        text += '\n';
    }
    if (!WriteTextAtomically(path, text))
        AC_LOG_DEBUG(kModule, "Cannot persist owner records to %s.", path.c_str());
}

// Applies one decision to an already-read manifest. `content` is updated in
// place; the record follows a successful write (and a write failure leaves the
// previous record valid).
void ApplyToManifest(const std::string& path, steam::AppId appId, const OwnerContext& ctx, const OwnerPlan& plan,
                     std::string& content, OwnerRecords& records, bool& recordsDirty, OwnerPolicyStats& stats) {
    const auto existing = records.find(appId);
    const bool hasRecord = existing != records.end();

    if (plan.action == OwnerAction::Skip) {
        ++stats.unchanged;
        if (plan.dropRecord && hasRecord) {
            records.erase(existing);
            recordsDirty = true;
        }
        return;
    }

    const bool changed = plan.action == OwnerAction::Set ? SetLastOwner(content, plan.owner) : RemoveLastOwner(content);
    if (changed) {
        if (!WriteTextAtomically(path, content)) {
            ++stats.errors;
            AC_LOG_WARN(kModule, "Cannot rewrite appmanifest_%u.acf (locked or read-only).", appId);
            return;
        }
        if (plan.action == OwnerAction::Set) {
            ++stats.applied;
            AC_LOG_INFO(kModule, "App %u: LastOwner set to %llu.", appId, static_cast<unsigned long long>(plan.owner));
        } else {
            ++stats.removed;
            AC_LOG_INFO(kModule, "App %u: LastOwner dropped.", appId);
        }
    } else {
        ++stats.unchanged;
    }

    if (plan.dropRecord) {
        if (hasRecord) {
            records.erase(appId);
            recordsDirty = true;
        }
        return;
    }

    const std::uint64_t written = plan.action == OwnerAction::Set ? plan.owner : 0;
    const OwnerRecord previous = hasRecord ? existing->second : OwnerRecord{};
    records[appId] = RecordAfterWrite(previous, hasRecord, ctx.currentLastOwner, written);
    recordsDirty = true;
}

} // namespace

OwnerPolicyStats ApplyInstalledAppOwners() {
    OwnerPolicyStats stats;

    const auto settings = Settings::Snapshot();
    const bool bannerEnabled = settings && settings->libraryOwnerBanner;
    const std::uint64_t bannerSteamId = bannerEnabled ? settings->libraryOwnerBannerSteamId : 0;
    std::unordered_set<steam::AppId> bannerApps;
    if (bannerEnabled) {
        bannerApps.reserve(settings->libraryOwnerBannerApps.size());
        for (const steam::AppId appId : settings->libraryOwnerBannerApps)
            bannerApps.insert(appId);
    }

    std::lock_guard<std::mutex> passLock(s_passMutex);
    OwnerRecords records = LoadRecords();
    bool recordsDirty = false;
    // One login owns the whole pass; reading it per manifest would hit the
    // registry once per app for the same answer.
    const std::uint64_t activeSteamId = steamid::GetActiveSteamId64();

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

            // Apps Aether spoofs now, apps Steam reports as family-shared, plus
            // apps Aether wrote earlier (their .lua may since have been
            // removed). A manifest with no record and no classification is a
            // real Steam record: skip it before reading anything.
            const bool managed = luadata::HasDepot(appId);
            const bool familyShared = luadata::IsFamilyShared(appId);
            const auto record = records.find(appId);
            if (!managed && !familyShared && record == records.end())
                continue;

            ++stats.scanned;
            const std::string path = ac::strings::Narrow(it->path().wstring());
            std::string content;
            if (!ReadTextFile(path, content)) {
                ++stats.errors;
                AC_LOG_DEBUG(kModule, "Cannot read appmanifest_%u.acf; leaving it untouched.", appId);
                continue;
            }

            OwnerContext ctx;
            ctx.bannerEnabled = bannerEnabled;
            ctx.managed = managed;
            ctx.familyShared = familyShared;
            ctx.listed = bannerApps.count(appId) != 0;
            ctx.bannerSteamId = bannerSteamId;
            ctx.activeSteamId = activeSteamId;
            ctx.recorded = record != records.end();
            if (ctx.recorded)
                ctx.record = record->second;
            ctx.currentLastOwner = ReadLastOwner(content);

            const OwnerPlan plan = PlanOwnerAction(ctx);
            ApplyToManifest(path, appId, ctx, plan, content, records, recordsDirty, stats);
        }
    }

    if (recordsDirty)
        SaveRecords(records);

    if (stats.applied != 0 || stats.removed != 0 || stats.errors != 0) {
        AC_LOG_INFO(kModule, "Owner policy: scanned=%zu set=%zu removed=%zu unchanged=%zu errors=%zu.", stats.scanned,
                    stats.applied, stats.removed, stats.unchanged, stats.errors);
    } else {
        AC_LOG_DEBUG(kModule, "Owner policy: scanned=%zu, nothing to change.", stats.scanned);
    }
    return stats;
}

void RequestOwnerApply() {
    bool expected = false;
    if (!s_pending.compare_exchange_strong(expected, true, std::memory_order_acq_rel))
        return;
    if (!workers::Submit([] {
            s_pending.store(false, std::memory_order_release);
            ApplyInstalledAppOwners();
        })) {
        s_pending.store(false, std::memory_order_release);
    }
}

void ApplyOwnersAtStartup() {
    if (!workers::Submit([] { ApplyInstalledAppOwners(); })) {
        AC_LOG_DEBUG(kModule, "Startup owner pass skipped: worker queue unavailable.");
    }
}

} // namespace ac::appmanifest
