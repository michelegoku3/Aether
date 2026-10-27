#pragma once

#include <chrono>
#include <cstdint>
#include <functional>
#include <mutex>
#include <string>
#include <unordered_set>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// DonorProbe — walks the whole donor pool for an app whose first donor had no
// data, instead of waiting for the Steam client to ask again.
//
// Why: the client asks GetUserStats once or twice per session, and each answer
// only advances DonorPool by one slot, so a game owned by a donor late in the
// pool never receives its achievement schema. The probe composes the remaining
// requests itself: the app is learned in one session, persisted (DonorPool's
// learning snapshot) and the schema is published into Steam's cache
// (SchemaCache), so the client serves it from then on.
//
// The wire sender and the schema sink are injected, which keeps the state
// machine free of protobuf/Win32 and testable without Steam.
// ---------------------------------------------------------------------------
namespace ac::hooks::wire::donorprobe {

// One GetUserStats service call for (appId, donor). Returns the jobid used for
// the request (0 when the frame could not be sent).
using Sender = std::function<std::uint64_t(steam::AppId appId, std::size_t donor)>;

// Publishes a schema found by the walk. Returns true when it is in place.
using Sink = std::function<bool(steam::AppId appId, const std::string& schema)>;

// How long a pool nobody answered from is left alone before it is walked again.
constexpr std::int64_t kRetryWindowSeconds = 7 * 24 * 60 * 60;

// Minimum spacing between probe requests. The walk is response-driven, so this
// only matters when the CM answers faster than the gap: pacing keeps a discovery
// list of ~100 donors from turning into a burst.
constexpr std::chrono::milliseconds kMinProbeGap{250};

// EResult values that mean "slow down" (k_EResultLimitExceeded,
// k_EResultRateLimitExceeded): the walk stops for the session instead of
// hammering the CM.
constexpr bool IsRateLimited(std::int32_t eresult) { return eresult == 25 || eresult == 84; }

// Wait left before the next probe may be sent (0 when the gap already elapsed).
std::chrono::milliseconds ProbeDelay(std::chrono::steady_clock::time_point lastSend,
                                     std::chrono::steady_clock::time_point now);

// Runs an action after `delay`. Boot wires a worker-backed scheduler so the
// pacing never sleeps on Steam's network thread; when unset (tests) the action
// runs immediately.
using Scheduler = std::function<void(std::chrono::milliseconds delay, std::function<void()> action)>;
void SetScheduler(Scheduler scheduler);

// Probe runtime data. It lives in AetherCoreState (central-state rule): the
// injected wire sender/sink and the in-flight walks. The ticket that this module
// owns is the walk bookkeeping only; donor learning stays in DonorPool.
struct State {
    mutable std::mutex mutex;
    Sender sender;
    Sink sink;
    Scheduler scheduler;
    std::unordered_set<steam::AppId> walking;
    std::chrono::steady_clock::time_point lastSend{};
    bool throttled = false;
};

void SetSender(Sender sender);
void SetSink(Sink sink);

// A donor answered without data for this app: start a walk if none is running
// and the pool was not exhausted within the retry window. Returns true when a
// probe request was sent.
bool OnDonorWithoutData(steam::AppId appId);

// Result of a probe request. Pins the donor and publishes the schema when it
// carries data, otherwise continues with the next untried donor. `eresult` is
// the header's answer: a rate-limit stops every walk for the session.
// Returns true when a schema was found and published.
bool OnProbeResult(steam::AppId appId, std::size_t donor, bool okWithData, const std::string& schema,
                   std::int32_t eresult);

// True while a probe request is in flight for this app.
bool IsWalking(steam::AppId appId);

// True once the CM answered with a rate-limit: no further walk this session.
bool IsThrottled();

} // namespace ac::hooks::wire::donorprobe
