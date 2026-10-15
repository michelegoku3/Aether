#include "pch.h"
#include "hooks/wire/DonorPool.h"

#include <deque>
#include <mutex>
#include <unordered_map>

#include "core/AetherCoreState.h"
#include "core/Logger.h"

namespace ac::hooks::DonorPool {
namespace {

constexpr const char* kModule = "Wire.Achievement";

// 15 SteamID64 inherited from LumaCore for the fallback pool (byte-identical).
constexpr std::uint64_t kLumaCoreStatSteamIdPool[kPoolCount] = {
    76561198017975643ULL, 76561198001678750ULL, 76561198355953202ULL, 76561197979911851ULL, 76561198040673812ULL,
    76561198367471798ULL, 76561198028125071ULL, 76561198012616627ULL, 76561197971398453ULL, 76561197977849691ULL,
    76561198019373005ULL, 76561198155124847ULL, 76561198063534772ULL, 76561198072711049ULL, 76561198028121353ULL,
};

// LumaCore's default/primary stat SteamID (kDefaultStatSteamId) is the LAST
// entry of the pool: 76561198028121353 == pool[14]. LumaCore's
// DefaultPoolIndex() searches the pool for this ID and tries it FIRST on the
// very first request for an app. Aether previously started at pool[0], so for
// a game owned only by that donor (e.g. Endacopia) it would cycle 0,1,2,... and
// the game would give up long before ever reaching index 14. We match LumaCore
// exactly by making pool[14] the starting point for every new app.
// (kDefaultPoolIndex itself lives in DonorPool.h: PoolEntry defaults to it.)

constexpr auto kAttemptWindow = std::chrono::seconds(15);
constexpr std::size_t kAttemptCap = 24;

constexpr auto kPendingWindow = std::chrono::seconds(30);

// Central-state accessor: all runtime data lives in
// g_state.achievements.donor (real AetherCoreState, or the test seam's
// identical struct). Per-group locks from SharedState keep the historical
// locking discipline unchanged.
SharedState& S() { return g_state.achievements.donor; }

void PruneAttemptsLocked(Clock::time_point now) {
    SharedState& s = S();
    std::erase_if(s.jobIdToAttempt, [&now](const auto& e) { return now - e.second.seen > std::chrono::seconds(30); });
    for (auto it = s.recentAttempts.begin(); it != s.recentAttempts.end();) {
        if (now - it->seen > kAttemptWindow)
            it = s.recentAttempts.erase(it);
        else
            ++it;
    }
    while (s.recentAttempts.size() > kAttemptCap)
        s.recentAttempts.pop_front();
}

} // namespace

std::uint64_t DonorSteamId(std::size_t index) { return kLumaCoreStatSteamIdPool[index % kPoolCount]; }

std::size_t PickIndex(steam::AppId appId) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    auto& e = s.pool[appId];
    return e.hasPreferred ? e.preferred : e.next;
}

void NoteAttemptResult(steam::AppId appId, std::size_t index, bool okWithData) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    auto& e = s.pool[appId];
    if (okWithData) {
        e.preferred = index;
        e.hasPreferred = true;
        e.next = index;
        AC_LOG_INFO_ONCE(kModule, "Pool AppID %u: preferred index %zu (donor %llu has data).", appId, index,
                         kLumaCoreStatSteamIdPool[index]);
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
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.attemptMutex);
    PruneAttemptsLocked(now);
    a.sequence = s.nextSequence++;
    if (hasJobId)
        s.jobIdToAttempt[jobId] = a;
    s.recentAttempts.push_back(a);
}

Correl ResolveAttempt(bool hasJobIdTarget, std::uint64_t jobIdTarget, StatAttempt& out) {
    auto now = Clock::now();
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.attemptMutex);
    PruneAttemptsLocked(now);

    if (hasJobIdTarget) {
        auto it = s.jobIdToAttempt.find(jobIdTarget);
        if (it != s.jobIdToAttempt.end()) {
            out = it->second;
            s.jobIdToAttempt.erase(it);
            // Successful correlation: also drop the attempt from the fallback
            // queue, otherwise it lingers as a "ghost" for kAttemptWindow and
            // makes the next response's fallback ambiguous (n>1), which would
            // then reach the game UNREWRITTEN (with the donor's stats). Flagged
            // by the 21/08/2026 external review.
            for (auto itDq = s.recentAttempts.begin(); itDq != s.recentAttempts.end();) {
                if (itDq->sequence == out.sequence)
                    itDq = s.recentAttempts.erase(itDq);
                else
                    ++itDq;
            }
            AC_LOG_DEBUG(kModule, "Response correlation via jobid_target %llu -> attempt AppID %u (pool %zu).",
                         static_cast<unsigned long long>(jobIdTarget), out.appId, out.poolIndex);
            return Correl::Resolved;
        }
    }
    // Fallback: a single recent in-flight request for this pipe.
    StatAttempt cand;
    std::size_t n = 0;
    for (const auto& a : s.recentAttempts) {
        if (now - a.seen <= kAttemptWindow) {
            cand = a;
            ++n;
        }
    }
    if (n == 1) {
        out = cand;
        for (auto it = s.recentAttempts.begin(); it != s.recentAttempts.end();) {
            if (it->sequence == cand.sequence)
                it = s.recentAttempts.erase(it);
            else
                ++it;
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
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.pendingMutex);
    auto now = Clock::now();
    std::erase_if(s.pendingClientStats, [&now](const auto& e) { return now - e.second.seen > kPendingWindow; });
    s.pendingClientStats[appId] = attempt;
}

bool TakePendingClientStats(steam::AppId appId, StatAttempt& out) {
    auto now = Clock::now();
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.pendingMutex);
    std::erase_if(s.pendingClientStats, [&now](const auto& e) { return now - e.second.seen > kPendingWindow; });
    auto it = s.pendingClientStats.find(appId);
    if (it == s.pendingClientStats.end())
        return false;
    out = it->second;
    s.pendingClientStats.erase(it);
    return true;
}

} // namespace ac::hooks::DonorPool
