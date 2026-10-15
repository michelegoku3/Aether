#pragma once

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <mutex>
#include <unordered_map>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// DonorPool — the LumaCore-style self-learning donor pool and the
// send->recv correlation state for the UserStats spoofing pipeline
// (P9 extraction from AchievementModule).
//
// Responsibilities:
//   * the 15-donor SteamID pool + per-app learning (PickIndex remembers the
//     first donor that answered WITH data and retries from there);
//   * attempt correlation for the service path (151/152, jobid-based with a
//     single-recent-attempt fallback);
//   * pending correlation for the client path (818/819, per-app with TTL).
//
// No protobuf here: callers extract jobids from headers themselves, so this
// module stays a pure state machine (headers keep the wire types out).
// ---------------------------------------------------------------------------
namespace ac::hooks::DonorPool {

using Clock = std::chrono::steady_clock;

constexpr std::size_t kPoolCount = 15;

// Starting donor for every new app (matches the legacy default exactly;
// see DonorPool.cpp for why pool[14] and not pool[0]).
constexpr std::size_t kDefaultPoolIndex = 14;

// SteamID64 of pool slot `index` (index < kPoolCount).
std::uint64_t DonorSteamId(std::size_t index);

// Donor to use for the next spoofed request for this app.
std::size_t PickIndex(steam::AppId appId);

// Learns from the donor's answer: okWithData pins the donor as preferred;
// an empty answer advances the round-robin (and drops a stale preference).
void NoteAttemptResult(steam::AppId appId, std::size_t index, bool okWithData);

struct StatAttempt {
    steam::AppId appId = 0;
    std::size_t poolIndex = 0;
    std::uint64_t sequence = 0;
    Clock::time_point seen{};
};

// Correlation verdict between a response and the requests we spoofed.
// NoMatch = response to a request we did NOT spoof (pass through untouched).
// Ambiguous = multiple spoofed requests in flight, cannot attribute: the
// payload almost certainly belongs to the donor and must be stripped
// (see the "The Fool" Cyberpunk leak, 2026-08-21).
enum class Correl { Resolved, Ambiguous, NoMatch };

// Per-app donor learning entry. Stored in central state (AetherCoreState),
// never as a module-local global.
struct PoolEntry {
    std::size_t next = kDefaultPoolIndex;
    std::size_t preferred = 0;
    bool hasPreferred = false;
};

// Central-state home for all DonorPool runtime data: per-app learning,
// service-path correlation and client-path pending entries, each with its
// own lock. Both the real AetherCoreState and the test seam embed this
// exact struct, so tests exercise the real ownership.
struct SharedState {
    mutable std::mutex poolMutex;
    std::unordered_map<steam::AppId, PoolEntry> pool;
    mutable std::mutex attemptMutex;
    std::unordered_map<std::uint64_t, StatAttempt> jobIdToAttempt; // jobid_source -> attempt
    std::deque<StatAttempt> recentAttempts;
    std::uint64_t nextSequence = 1;
    mutable std::mutex pendingMutex;
    std::unordered_map<steam::AppId, StatAttempt> pendingClientStats;
};

// Service path (151/152): remember the attempt keyed by jobid_source when
// available, with a bounded recent-attempt fallback otherwise.
void RecordAttempt(StatAttempt a, bool hasJobId, std::uint64_t jobId);
Correl ResolveAttempt(bool hasJobIdTarget, std::uint64_t jobIdTarget, StatAttempt& out);

// Client path (818/819): per-app pending with TTL.
void RecordPendingClientStats(steam::AppId appId, StatAttempt attempt);
bool TakePendingClientStats(steam::AppId appId, StatAttempt& out);

} // namespace ac::hooks::DonorPool
