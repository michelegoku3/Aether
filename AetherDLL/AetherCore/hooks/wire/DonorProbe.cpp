#include "pch.h"
#include "hooks/wire/DonorProbe.h"

#include <chrono>
#include <utility>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/wire/DonorPool.h"

namespace ac::hooks::wire::donorprobe {
namespace {

constexpr const char* kModule = "Wire.Achievement";

State& S() { return g_state.achievements.probe; }

std::uint64_t UnixNow() {
    return static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch()).count());
}

// Reserves the walk slot and hands out a copy of the sender. False when no
// sender is wired or a probe for this app is already in flight.
bool Reserve(steam::AppId appId, Sender& sender, Scheduler& scheduler, std::chrono::milliseconds& delay) {
    std::lock_guard<std::mutex> lock(S().mutex);
    if (S().throttled || !S().sender)
        return false;
    if (!S().walking.insert(appId).second)
        return false;
    sender = S().sender;
    scheduler = S().scheduler;
    const auto now = std::chrono::steady_clock::now();
    delay = ProbeDelay(S().lastSend, now);
    return true;
}

void Release(steam::AppId appId) {
    std::lock_guard<std::mutex> lock(S().mutex);
    S().walking.erase(appId);
}

// Sends the probe for `donor`, honouring the pacing gap, and drops the
// reservation when the frame cannot be built (the next donor failure is then
// free to start a new walk).
bool SendProbe(steam::AppId appId, std::size_t donor, const Sender& sender, const Scheduler& scheduler,
               std::chrono::milliseconds delay) {
    if (!sender) {
        Release(appId);
        return false;
    }
    const auto send = [appId, donor, sender]() {
        {
            std::lock_guard<std::mutex> lock(S().mutex);
            S().lastSend = std::chrono::steady_clock::now();
        }
        if (sender(appId, donor) == 0) {
            Release(appId);
            return false;
        }
        return true;
    };
    if (delay.count() <= 0 || !scheduler)
        return send();
    // Deferred: the walk slot stays reserved until the job runs.
    scheduler(delay, send);
    return true;
}

} // namespace

std::chrono::milliseconds ProbeDelay(std::chrono::steady_clock::time_point lastSend,
                                     std::chrono::steady_clock::time_point now) {
    if (lastSend.time_since_epoch().count() == 0)
        return std::chrono::milliseconds{0};
    const auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(now - lastSend);
    return elapsed >= kMinProbeGap ? std::chrono::milliseconds{0} : kMinProbeGap - elapsed;
}

void SetSender(Sender sender) {
    std::lock_guard<std::mutex> lock(S().mutex);
    S().sender = std::move(sender);
}

void SetSink(Sink sink) {
    std::lock_guard<std::mutex> lock(S().mutex);
    S().sink = std::move(sink);
}

void SetScheduler(Scheduler scheduler) {
    std::lock_guard<std::mutex> lock(S().mutex);
    S().scheduler = std::move(scheduler);
}

bool IsWalking(steam::AppId appId) {
    std::lock_guard<std::mutex> lock(S().mutex);
    return S().walking.contains(appId);
}

bool IsThrottled() {
    std::lock_guard<std::mutex> lock(S().mutex);
    return S().throttled;
}

bool OnDonorWithoutData(steam::AppId appId) {
    Sender sender;
    Scheduler scheduler;
    std::chrono::milliseconds delay{0};
    if (!Reserve(appId, sender, scheduler, delay))
        return false;

    // A pool already walked to the end stays quiet until the retry window has
    // elapsed, so an app no donor owns costs nothing per session.
    const std::uint64_t exhaustedAt = DonorPool::PoolExhaustedAt(appId);
    if (exhaustedAt != 0) {
        const std::uint64_t now = UnixNow();
        if (now <= exhaustedAt || now - exhaustedAt < static_cast<std::uint64_t>(kRetryWindowSeconds)) {
            Release(appId);
            return false;
        }
        DonorPool::ResetWalk(appId);
    }

    const std::size_t count = DonorPool::PoolCount();
    const std::size_t next = DonorPool::NextUntried(appId);
    if (next >= count) {
        DonorPool::NotePoolExhausted(appId, UnixNow());
        Release(appId);
        return false;
    }

    AC_LOG_DEBUG(kModule, "Donor probe: AppID %u tries donor slot %zu of %zu.", appId, next, count);
    return SendProbe(appId, next, sender, scheduler, delay);
}

bool OnProbeResult(steam::AppId appId, std::size_t donor, bool okWithData, const std::string& schema,
                   std::int32_t eresult) {
    Release(appId);

    if (IsRateLimited(eresult)) {
        // The CM is telling us to slow down: stop every walk for this session
        // (the cursor is persisted, so the next session resumes where we left).
        {
            std::lock_guard<std::mutex> lock(S().mutex);
            S().throttled = true;
        }
        AC_LOG_WARN(kModule,
                    "Donor probe: Steam answered eresult %d (rate limit) for AppID %u: walks paused until the next "
                    "Steam start.",
                    eresult, appId);
        return false;
    }

    if (okWithData) {
        // The donor keeps supplying the schema for this app on every future
        // client request, so pin it before publishing.
        DonorPool::NoteAttemptResult(appId, donor, /*okWithData=*/true);
        Sink sink;
        {
            std::lock_guard<std::mutex> lock(S().mutex);
            sink = S().sink;
        }
        const bool stored = sink && sink(appId, schema);
        AC_LOG_INFO(kModule, "Donor probe: AppID %u schema found on donor %llu (slot %zu), %s.", appId,
                    DonorPool::DonorSteamId(donor), donor, stored ? "stored" : "not stored");
        return true;
    }

    DonorPool::NoteAttemptResult(appId, donor, /*okWithData=*/false);

    const std::size_t count = DonorPool::PoolCount();
    const std::size_t next = DonorPool::NextUntried(appId);
    if (next >= count) {
        DonorPool::NotePoolExhausted(appId, UnixNow());
        AC_LOG_INFO(kModule,
                    "Donor probe: AppID %u exhausted after %zu donors, no schema; next attempt after the retry "
                    "window.",
                    appId, count);
        return false;
    }

    Sender sender;
    Scheduler scheduler;
    std::chrono::milliseconds delay{0};
    if (Reserve(appId, sender, scheduler, delay))
        SendProbe(appId, next, sender, scheduler, delay);
    return false;
}

} // namespace ac::hooks::wire::donorprobe
