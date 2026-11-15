#include "pch.h"
#include "hooks/wire/SteamStatsCache.h"

#include <bit>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <functional>
#include <mutex>
#include <unordered_map>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/wire/AchievementUserState.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/SchemaBuckets.h"
#include "hooks/wire/UserStatsSnapshot.h"
#include "utils/KeyValues.h"
#include "utils/WinFile.h"

namespace ac::backup::statscache {
namespace {

constexpr const char* kModule = "Wire.Achievement";

// ---------------------------------------------------------------------------
// Copy of the Steam cache .bins (account stats + schema).
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// SCHEMA-DRIVEN interpretation of bitfield stats.
//
// The safe way to know whether a stat is an achievement bucket is reading
// the game schema (appcache\stats\UserGameStatsSchema_<appid>.bin): the
// "stats" section holds one dictionary per bucket, and buckets with a
// "bits" sub-dictionary are achievement bitfields. This removes every
// heuristic: Stanley uses bucket 3 with bits 0-4 (ignored by the fast-log
// "bit >= 8" threshold), Spider-Man uses buckets 0/200/201.
//
// Per-app cache shared by the 5466 path (Steam's network thread) and the backup
// worker, so every access goes through g_schemaMutex. SchemaBucketsFor hands out
// a copy: a set can be replaced in the same session when a missing schema
// appears, so a reference into the map would not stay valid across threads.
// ---------------------------------------------------------------------------
std::mutex g_schemaMutex;
std::unordered_map<steam::AppId, std::unordered_set<std::uint32_t>> g_schemaBuckets;
std::unordered_set<steam::AppId> g_schemaParsed;

std::unordered_set<std::uint32_t> ParseSchemaBuckets(const std::string& path) {
    std::ifstream f(winfile::ToPath(path), std::ios::binary);
    if (!f.is_open())
        return {};
    std::vector<std::uint8_t> buf((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
    detail::SchemaBucketWalker walker;
    kv1::WalkBinary(buf.data(), buf.size(), walker);
    return std::move(walker.buckets);
}

// Caller holds g_schemaMutex. A missing schema is NOT remembered as parsed: the
// file can appear later in the same session (the library page transcribes one),
// and the next lookup must then see the real bucket set.
const std::unordered_set<std::uint32_t>& SchemaBucketsLocked(steam::AppId appId) {
    if (!g_schemaParsed.contains(appId)) {
        const std::string path =
            g_state.steamInstallPath + "\\appcache\\stats\\UserGameStatsSchema_" + std::to_string(appId) + ".bin";
        if (winfile::Exists(path)) {
            g_schemaBuckets[appId] = ParseSchemaBuckets(path);
            g_schemaParsed.insert(appId);
            AC_LOG_DEBUG(kModule, "Backup: schema for AppID %u -> %zu achievement bucket(s).", appId,
                         g_schemaBuckets[appId].size());
        }
    }
    return g_schemaBuckets[appId];
}

// ---------------------------------------------------------------------------
// "Richness" evaluation of a UserGameStats file: counts achievements as the
// sum of set bits in each bucket's `data` bitfields (binary VDF format:
// 0x00=dict, 0x01=string, 0x02=int32, 0x08=dict close).
// Feeds the monotone guard: never overwrite a backup with a poorer cache
// (e.g. just wiped by login-reconcile). -1 = unreadable.
// ---------------------------------------------------------------------------

int CountDictAchievements(const std::vector<std::uint8_t>& buf, std::size_t& pos, int depth) {
    int total = 0;
    while (pos < buf.size()) {
        const std::uint8_t type = buf[pos++];
        if (type == 0x08)
            return total;
        std::size_t keyEnd = pos;
        while (keyEnd < buf.size() && buf[keyEnd] != 0x00)
            ++keyEnd;
        if (keyEnd >= buf.size())
            return total;
        const std::string key(reinterpret_cast<const char*>(&buf[pos]), keyEnd - pos);
        pos = keyEnd + 1;
        if (type == 0x00) {
            total += CountDictAchievements(buf, pos, depth + 1);
        } else if (type == 0x01) {
            std::size_t valEnd = pos;
            while (valEnd < buf.size() && buf[valEnd] != 0x00)
                ++valEnd;
            pos = valEnd + 1;
        } else if (type == 0x02) {
            if (pos + 4 > buf.size())
                return total;
            if (key == "data") {
                std::uint32_t bits = 0;
                for (int b = 0; b < 4; ++b)
                    bits |= static_cast<std::uint32_t>(buf[pos + b]) << (8 * b);
                total += std::popcount(bits);
            }
            pos += 4;
        } else if (type == 0x03) {
            pos += 4;
        } else {
            return total; // unknown type: stop (unexpected format)
        }
    }
    return total;
}

int CountAchievementsInBinFileImpl(const std::string& path) {
    std::ifstream f(winfile::ToPath(path), std::ios::binary);
    if (!f.is_open())
        return -1;
    std::vector<std::uint8_t> buf((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
    std::size_t pos = 0;
    return CountDictAchievements(buf, pos, 0);
}

// Copies the Steam cache .bins (account stats + schema). `force` skips the
// rate-limit (used for shutdown final copies); otherwise the worker limits
// copies of the same (app, account) to one every kBinCopyMinInterval so it
// does not copy the file on every game-saved stat.
constexpr auto kBinCopyMinInterval = std::chrono::seconds(60);

// Worker-exclusive ownership: last copy instant per (app, account).
std::unordered_map<std::uint64_t, std::chrono::steady_clock::time_point> g_lastBinCopy;

void BackupStatsBinsImpl(steam::AppId appId, std::uint32_t accountId, bool force = false) {
    const std::string statsDir = g_state.steamInstallPath + "\\appcache\\stats";
    const std::string dstDir = io::BackupDirForApp(appId);
    if (dstDir.empty())
        return; // backup unavailable (no AetherData)

    if (!force) {
        const std::uint64_t key = (static_cast<std::uint64_t>(appId) << 32) | accountId;
        const auto now = std::chrono::steady_clock::now();
        auto it = g_lastBinCopy.find(key);
        if (it != g_lastBinCopy.end() && now - it->second < kBinCopyMinInterval) {
            return; // recently copied: skip silently
        }
        g_lastBinCopy[key] = now;
    }

    const std::string userBin = "UserGameStats_" + std::to_string(accountId) + "_" + std::to_string(appId) + ".bin";
    const std::string schemaBin = "UserGameStatsSchema_" + std::to_string(appId) + ".bin";

    if (winfile::Exists(statsDir + "\\" + userBin)) {
        // MONOTONE GUARD: never overwrite a backup with a poorer cache. When
        // the Steam cache was just wiped (login-reconcile), the source file
        // holds FEWER achievements than the backup: then the backup is kept.
        // Backups can only grow, never regress.
        const std::string dstUserBin = dstDir + "\\" + userBin;
        if (winfile::Exists(dstUserBin)) {
            const int srcCount = CountAchievementsInBinFile(statsDir + "\\" + userBin);
            const int dstCount = CountAchievementsInBinFile(dstUserBin);
            if (srcCount >= 0 && dstCount >= 0 && srcCount < dstCount) {
                // The client reads this file at login and draws the game page's
                // achievements block from it, so a cache Steam emptied is what
                // makes the section disappear. Refusing to copy is not enough:
                // put the richer copy back where the client reads it.
                const std::string staging = io::UniqueStagingPath(statsDir + "\\" + userBin);
                if (winfile::CopyOverwrite(dstUserBin, staging) &&
                    io::AtomicReplace(staging, statsDir + "\\" + userBin)) {
                    AC_LOG_INFO(kModule,
                                "Backup: %s lost achievements (%d now, %d in the backup); the backup copy is back in "
                                "Steam's cache.",
                                userBin.c_str(), srcCount, dstCount);
                } else {
                    AC_LOG_WARN(kModule,
                                "Backup: %s lost achievements (%d now, %d in the backup) and the backup copy could "
                                "not be restored.",
                                userBin.c_str(), srcCount, dstCount);
                }
                // The schema is harmless to copy either way.
                if (winfile::Exists(statsDir + "\\" + schemaBin)) {
                    winfile::CopyOverwrite(statsDir + "\\" + schemaBin, dstDir + "\\" + schemaBin);
                }
                return;
            }
        }
        if (winfile::CopyOverwrite(statsDir + "\\" + userBin, dstUserBin)) {
            AC_LOG_INFO(kModule, "Backup: copied %s to %s.", userBin.c_str(), dstDir.c_str());
        } else {
            AC_LOG_WARN(kModule, "Backup: copy of %s failed.", userBin.c_str());
        }
    } else {
        AC_LOG_DEBUG(kModule, "Backup: %s not present yet in appcache\\stats.", userBin.c_str());
    }
    if (winfile::Exists(statsDir + "\\" + schemaBin)) {
        if (!winfile::CopyOverwrite(statsDir + "\\" + schemaBin, dstDir + "\\" + schemaBin)) {
            AC_LOG_WARN(kModule, "Backup: copy of %s failed.", schemaBin.c_str());
        }
    }
}

// Steam writes the unlock date into the stats file a moment AFTER the game's
// commit, so the snapshot written while handling that commit only knows the ids
// (unlock_time 0, "baseline"). The backup copy of the file is the dated one:
// reading it back is what keeps a rebuild from the JSON, and the achievement
// page's local fallback, from losing the real times.
//
// The backup copy is the source of truth here, not Steam's cache: the monotone
// guard may have just restored the richer copy into the cache, and the backup
// is the one that survives a wipe.
void SyncDatesFromStatsBin(steam::AppId appId, std::uint32_t accountId, const std::string& binPath) {
    const std::string snapshotPath = snapshot::SnapshotPath(appId, accountId);
    if (snapshotPath.empty())
        return;
    std::ifstream file(winfile::ToPath(binPath), std::ios::binary);
    if (!file.is_open())
        return;
    const std::vector<std::uint8_t> buf((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());

    // Rows collected before a truncation stay usable: a partial file can still
    // carry dates for the unlocks it holds.
    achieve::UserState state;
    achieve::ParseUserState(buf.data(), buf.size(), state);
    if (state.unlocked.empty())
        return;

    snapshot::SnapshotData snap = snapshot::Load(snapshotPath);
    if (snap.unlocks.empty())
        return;

    // Ids still waiting for a date: only these can be adopted, and naming them
    // after the merge is what ties the real time to the achievement in the log.
    std::vector<std::uint32_t> pending;
    for (const auto& entry : snap.unlocks)
        if (entry.unlockTime == 0)
            pending.push_back(entry.id);

    const std::size_t adopted = snapshot::MergeUnlockTimes(snap, state.unlocked);
    if (adopted == 0)
        return; // nothing changed: the snapshot file keeps its bytes and mtime

    // Individual SteamID64 derived from the account id: the snapshot header
    // keeps the same identity the commit path writes.
    constexpr std::uint64_t kIndividualBase = 76561197960265728ull;
    snapshot::Save(snapshotPath, appId, accountId, kIndividualBase + accountId, snap);
    for (const std::uint32_t id : pending) {
        for (const auto& entry : snap.unlocks) {
            if (entry.id != id || entry.unlockTime == 0)
                continue;
            AC_LOG_INFO(kModule, "Backup: achievement %u of app %u unlocked at %s (date read from Steam's stats file).",
                        id, appId, io::FormatUnixTime(entry.unlockTime).c_str());
        }
    }
    AC_LOG_DEBUG(kModule, "Backup: adopted %zu of %zu baseline unlock date(s) for app %u from %s.", adopted,
                 pending.size(), appId, binPath.c_str());
}

} // namespace

void BackupStatsBins(steam::AppId appId, std::uint32_t accountId, bool force) {
    BackupStatsBinsImpl(appId, accountId, force);
    // Dates live in the file, so they are repaired wherever the file is copied:
    // unlock, session start, periodic checkpoint, end of session, startup scan.
    const std::string dstDir = io::BackupDirForApp(appId);
    if (dstDir.empty())
        return;
    SyncDatesFromStatsBin(appId, accountId,
                          dstDir + "\\UserGameStats_" + std::to_string(accountId) + "_" + std::to_string(appId) +
                              ".bin");
}

int CountAchievementsInBinFile(const std::string& path) { return CountAchievementsInBinFileImpl(path); }

std::unordered_set<std::uint32_t> SchemaBucketsFor(steam::AppId appId) {
    std::lock_guard<std::mutex> lock(g_schemaMutex);
    return SchemaBucketsLocked(appId);
}

} // namespace ac::backup::statscache
