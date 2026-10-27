#pragma once

#include <bitset>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

#include "core/SteamTypes.h"
#include "hooks/wire/DonorIds.h"

// ---------------------------------------------------------------------------
// DonorPool — the LumaCore-style self-learning donor pool and the
// send->recv correlation state for the UserStats spoofing pipeline.
//
// Responsibilities:
//   * the donor SteamID pool + per-app learning (PickIndex remembers the first
//     donor that answered WITH data and retries from there);
//   * attempt correlation for the service path (151/152, jobid-based with a
//     single-recent-attempt fallback);
//   * pending correlation for the client path (818/819, per-app with TTL).
//
// The pool is compiled in (DonorIds.h): a game whose owner is missing from the
// list is fixed by appending that SteamID in a release, so every install ships
// the same donors and there is no per-user file to get wrong.
//
// No protobuf here: callers extract jobids from headers themselves, so this
// module stays a pure state machine (headers keep the wire types out).
// ---------------------------------------------------------------------------
namespace ac::hooks::DonorPool {

using Clock = std::chrono::steady_clock;

// Hard ceiling for the tried mask: it is a fixed-size bitset per app, so the
// pool size is bounded at compile time. Growing past it means widening the mask
// (and its hex width in donors.txt).
constexpr std::size_t kMaxPoolCount = 128;

// Starting donor for every new app (compiled slot 14; see DonorPool.cpp for why
// not slot 0).
constexpr std::size_t kDefaultPoolIndex = 14;

// Number of donors in the compiled pool.
std::size_t PoolCount();

// SteamID64 of pool slot `index` (wraps around PoolCount()).
std::uint64_t DonorSteamId(std::size_t index);

// Fingerprint of the compiled pool. Persisted with the learning: when a release
// changes the pool (a donor is added, removed or reordered) the per-app learning
// is stale and is discarded, so the new donor is actually tried.
std::uint64_t PoolFingerprint();

// Donor to use for the next spoofed request for this app.
std::size_t PickIndex(steam::AppId appId);

// Next slot this app has not tried yet, i.e. the next donor worth probing;
// PoolCount() when the walk is over (a donor answered with data or every slot
// already answered without data). The probe chain uses it so a donor that
// already answered without data is never asked twice.
std::size_t NextUntried(steam::AppId appId);

// Learns from the donor's answer: okWithData pins the donor as preferred;
// an empty answer marks the slot tried and advances the round-robin (and drops
// a stale preference).
void NoteAttemptResult(steam::AppId appId, std::size_t index, bool okWithData);

// Text snapshot of the per-app learning, so a Steam restart resumes from the
// donor that worked instead of walking the pool again. Empty pool -> "".
std::string SerializeLearning();

// Applies a snapshot. replace=true discards the current learning first (boot
// with the persisted file); false merges, with the text winning per app.
// A snapshot taken with a different pool fingerprint is discarded (replace) or
// merged as empty (no fingerprint match): donor indices are only meaningful for
// the pool that produced them.
void LoadLearning(const std::string& text, bool replace);

// Persistence used across Steam restarts: `<aethercore>\donors.txt`.
// SaveLearning refuses an empty path; LoadLearningFile keeps the in-memory
// learning when the file is missing or unreadable (first run).
bool SaveLearning(const std::string& path);
void LoadLearningFile(const std::string& path);

// Wall-clock stamp of the last exhausted walk (0 = never exhausted). Persisted
// so an app nobody owns is retried after a window instead of costing a full
// walk on every Steam restart.
void NotePoolExhausted(steam::AppId appId, std::uint64_t unixSeconds);
std::uint64_t PoolExhaustedAt(steam::AppId appId);

// Starts a new walk: clears the tried mask and the exhaustion stamp and puts
// the cursor back where a fresh app starts.
void ResetWalk(steam::AppId appId);

struct StatAttempt {
    steam::AppId appId = 0;
    std::size_t poolIndex = 0;
    std::uint64_t sequence = 0;
    Clock::time_point seen{};
    // True for requests this DLL composes itself (DonorProbe): resolved by
    // jobid only, never through the jobid-less fallback, and their response is
    // consumed internally instead of being forwarded to the client.
    bool probe = false;
};

// Correlation verdict between a response and the requests we spoofed.
// NoMatch = response to a request we did NOT spoof (pass through untouched).
// Ambiguous = multiple spoofed requests in flight, cannot attribute: the
// payload almost certainly belongs to the donor and must be stripped
// (see the "The Fool" Cyberpunk leak).
enum class Correl { Resolved, Ambiguous, NoMatch };

// Per-app donor learning entry. Stored in central state (AetherCoreState),
// never as a module-local global.
struct PoolEntry {
    std::size_t next = kDefaultPoolIndex;
    std::size_t preferred = 0;
    bool hasPreferred = false;
    // Bit i set = slot i already answered without data in this walk.
    std::bitset<kMaxPoolCount> tried;
    // Wall-clock (unix seconds) of the last exhausted walk; 0 = never.
    std::uint64_t exhaustedAt = 0;
};

// Central-state home for all DonorPool runtime data: the active donor list,
// per-app learning, service-path correlation and client-path pending entries,
// each with its own lock. Both the real AetherCoreState and the test seam embed
// this exact struct, so tests exercise the real ownership.
struct SharedState {
    mutable std::mutex poolMutex;
    // The compiled donor list (DonorIds.h). Immutable after static init.
    std::vector<std::uint64_t> donors{kCompiledDonors.begin(), kCompiledDonors.end()};
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
