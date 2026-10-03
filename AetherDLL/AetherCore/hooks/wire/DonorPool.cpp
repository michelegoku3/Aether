#include "pch.h"
#include "hooks/wire/DonorPool.h"

#include <deque>
#include <mutex>
#include <unordered_map>

#include "core/Logger.h"

namespace ac::hooks::DonorPool {
namespace {

constexpr const char* kModule = "Wire.Achievement";

// 15 SteamID64 ereditati da LumaCore per il pool di fallback (byte-identical).
constexpr std::uint64_t kLumaCoreStatSteamIdPool[kPoolCount] = {
    76561198017975643ULL,
    76561198001678750ULL,
    76561198355953202ULL,
    76561197979911851ULL,
    76561198040673812ULL,
    76561198367471798ULL,
    76561198028125071ULL,
    76561198012616627ULL,
    76561197971398453ULL,
    76561197977849691ULL,
    76561198019373005ULL,
    76561198155124847ULL,
    76561198063534772ULL,
    76561198072711049ULL,
    76561198028121353ULL,
};

// LumaCore's default/primary stat SteamID (kDefaultStatSteamId) is the LAST
// entry of the pool: 76561198028121353 == pool[14]. LumaCore's
// DefaultPoolIndex() searches the pool for this ID and tries it FIRST on the
// very first request for an app. Aether previously started at pool[0], so for
// a game owned only by that donor (e.g. Endacopia) it would cycle 0,1,2,... and
// the game would give up long before ever reaching index 14. We match LumaCore
// exactly by making pool[14] the starting point for every new app.
constexpr std::size_t kDefaultPoolIndex = 14;

struct PoolEntry {
    std::size_t next = kDefaultPoolIndex;   // LumaCore starts at the default donor
    std::size_t preferred = 0;
    bool hasPreferred = false;
};

std::mutex g_poolMutex;
std::unordered_map<steam::AppId, PoolEntry> g_pool;

constexpr auto kAttemptWindow = std::chrono::seconds(15);
constexpr std::size_t kAttemptCap = 24;

std::mutex g_attemptMutex;
std::unordered_map<std::uint64_t, StatAttempt> g_jobIdToAttempt;   // jobid_source -> attempt
std::deque<StatAttempt> g_recentAttempts;
std::uint64_t g_nextSequence = 1;

constexpr auto kPendingWindow = std::chrono::seconds(30);
std::mutex g_pendingMutex;
std::unordered_map<steam::AppId, StatAttempt> g_pendingClientStats;

void PruneAttemptsLocked(Clock::time_point now) {
    std::erase_if(g_jobIdToAttempt, [&now](const auto& e) {
        return now - e.second.seen > std::chrono::seconds(30);
    });
    for (auto it = g_recentAttempts.begin(); it != g_recentAttempts.end();) {
        if (now - it->seen > kAttemptWindow) it = g_recentAttempts.erase(it);
        else ++it;
    }
    while (g_recentAttempts.size() > kAttemptCap) g_recentAttempts.pop_front();
}

}  // namespace

std::uint64_t DonorSteamId(std::size_t index) {
    return kLumaCoreStatSteamIdPool[index % kPoolCount];
}

std::size_t PickIndex(steam::AppId appId) {
    std::lock_guard<std::mutex> lock(g_poolMutex);
    auto& e = g_pool[appId];
    return e.hasPreferred ? e.preferred : e.next;
}

void NoteAttemptResult(steam::AppId appId, std::size_t index, bool okWithData) {
    std::lock_guard<std::mutex> lock(g_poolMutex);
    auto& e = g_pool[appId];
    if (okWithData) {
        e.preferred = index;
        e.hasPreferred = true;
        e.next = index;
        AC_LOG_INFO_ONCE(kModule, "Pool AppID %u: preferred index %zu (donor %llu has data).",
                    appId, index, kLumaCoreStatSteamIdPool[index]);
        return;
    }
    if (e.hasPreferred && e.preferred == index) {
        e.hasPreferred = false;
        e.preferred = 0;
    }
    e.next = (index + 1) % kPoolCount;
    AC_LOG_DEBUG(kModule, "Pool AppID %u: advancing index %zu -> %zu (donor has no data).", appId, index, e.next);
}

void RecordAttempt(StatAttempt a, bool hasJobId, std::uint64_t jobId) {
    auto now = Clock::now();
    a.seen = now;
    std::lock_guard<std::mutex> lock(g_attemptMutex);
    PruneAttemptsLocked(now);
    a.sequence = g_nextSequence++;
    if (hasJobId) g_jobIdToAttempt[jobId] = a;
    g_recentAttempts.push_back(a);
}

Correl ResolveAttempt(bool hasJobIdTarget, std::uint64_t jobIdTarget, StatAttempt& out) {
    auto now = Clock::now();
    std::lock_guard<std::mutex> lock(g_attemptMutex);
    PruneAttemptsLocked(now);

    if (hasJobIdTarget) {
        auto it = g_jobIdToAttempt.find(jobIdTarget);
        if (it != g_jobIdToAttempt.end()) {
            out = it->second;
            g_jobIdToAttempt.erase(it);
            // Correlazione riuscita: rimuovi l'attempt ANCHE dalla coda di
            // fallback, altrimenti resta come "fantasma" per kAttemptWindow e
            // rende ambiguo (n>1) il fallback della risposta successiva, che a
            // quel punto passerebbe al gioco NON riscritta (con le stats del
            // donor). Segnalato dalla revisione esterna del 21/08/2026.
            for (auto itDq = g_recentAttempts.begin(); itDq != g_recentAttempts.end();) {
                if (itDq->sequence == out.sequence) itDq = g_recentAttempts.erase(itDq);
                else ++itDq;
            }
            AC_LOG_DEBUG(kModule, "Response correlation via jobid_target %llu -> attempt AppID %u (pool %zu).",
                        static_cast<unsigned long long>(jobIdTarget), out.appId, out.poolIndex);
            return Correl::Resolved;
        }
    }
    // Fallback: a single recent in-flight request for this pipe.
    StatAttempt cand;
    std::size_t n = 0;
    for (const auto& a : g_recentAttempts) {
        if (now - a.seen <= kAttemptWindow) { cand = a; ++n; }
    }
    if (n == 1) {
        out = cand;
        for (auto it = g_recentAttempts.begin(); it != g_recentAttempts.end();) {
            if (it->sequence == cand.sequence) it = g_recentAttempts.erase(it);
            else ++it;
        }
        AC_LOG_DEBUG(kModule, "Response correlation without jobid: single recent attempt -> AppID %u (pool %zu).",
                    out.appId, out.poolIndex);
        return Correl::Resolved;
    }
    if (n > 1) {
        AC_LOG_WARN(kModule,
                    "Ambiguous correlation: %zu overlapping spoofed requests in flight. The response will "
                    "be stripped of the donor payload for safety (no leak to the game).",
                    n);
        return Correl::Ambiguous;
    }
    return Correl::NoMatch;
}

void RecordPendingClientStats(steam::AppId appId, StatAttempt attempt) {
    std::lock_guard<std::mutex> lock(g_pendingMutex);
    auto now = Clock::now();
    std::erase_if(g_pendingClientStats, [&now](const auto& e) {
        return now - e.second.seen > kPendingWindow;
    });
    g_pendingClientStats[appId] = attempt;
}

bool TakePendingClientStats(steam::AppId appId, StatAttempt& out) {
    auto now = Clock::now();
    std::lock_guard<std::mutex> lock(g_pendingMutex);
    std::erase_if(g_pendingClientStats, [&now](const auto& e) {
        return now - e.second.seen > kPendingWindow;
    });
    auto it = g_pendingClientStats.find(appId);
    if (it == g_pendingClientStats.end()) return false;
    out = it->second;
    g_pendingClientStats.erase(it);
    return true;
}

}  // namespace ac::hooks::DonorPool
