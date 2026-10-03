#pragma once

#include <chrono>
#include <cstddef>
#include <cstdint>

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

// Service path (151/152): remember the attempt keyed by jobid_source when
// available, with a bounded recent-attempt fallback otherwise.
void RecordAttempt(StatAttempt a, bool hasJobId, std::uint64_t jobId);
Correl ResolveAttempt(bool hasJobIdTarget, std::uint64_t jobIdTarget, StatAttempt& out);

// Client path (818/819): per-app pending with TTL.
void RecordPendingClientStats(steam::AppId appId, StatAttempt attempt);
bool TakePendingClientStats(steam::AppId appId, StatAttempt& out);

}  // namespace ac::hooks::DonorPool
