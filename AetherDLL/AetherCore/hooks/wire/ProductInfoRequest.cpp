#include "pch.h"
#include "hooks/wire/ProductInfoRequest.h"

#include <atomic>
#include <string>

#include "core/Constants.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "hooks/wire/PacketRouter.h"

#include "steam_messages.pb.h"

namespace ac::hooks::productinfo {
namespace {

constexpr const char* kModule = "Wire.ProductInfo";

// A send only fails before a connection is captured (or after the hook is
// gone). The retry budget covers a slow startup and then gives up loudly
// instead of hammering a dead pipe.
constexpr int kMaxAttempts = 10;
constexpr unsigned long kRetryDelayMs = 2000;

RequestQueue s_queue;
std::atomic<bool> s_flushPending{false};

void Flush(int attempt);

void ScheduleFlush(int attempt) {
    if (!workers::Submit([attempt] { Flush(attempt); }))
        AC_LOG_DEBUG(kModule, "PICS product info send skipped (workers shut down).");
}

void Flush(int attempt) {
    s_flushPending.store(false, std::memory_order_relaxed);
    std::vector<steam::AppId> ids;
    s_queue.Drain(ids);
    if (ids.empty())
        return;

    bool failed = false;
    for (const steam::AppId appId : ids) {
        // Product metadata is public, not license-gated: one app per request
        // keeps the reply small and served on the normal path even when the
        // batch job is stuck in Timeout.
        CMsgClientPICSProductInfoRequest req;
        req.add_apps()->set_appid(appId);
        std::string body;
        if (!req.SerializeToString(&body)) {
            AC_LOG_WARN(kModule, "PICS product info request serialize failed for app %u.", appId);
            continue;
        }
        if (SendClientFrame(constants::emsg::kClientPICSProductInfoRequest,
                            reinterpret_cast<const std::uint8_t*>(body.data()),
                            static_cast<std::uint32_t>(body.size()))) {
            AC_LOG_INFO(kModule, "Requested PICS product info for app %u.", appId);
            continue;
        }
        failed = true;
        s_queue.Requeue(appId);
    }

    if (!failed)
        return;
    if (attempt + 1 >= kMaxAttempts) {
        AC_LOG_WARN_ONCE(kModule, "PICS product info send gave up after %d attempts; Steam retries on its own.",
                         kMaxAttempts);
        return;
    }
    AC_LOG_DEBUG(kModule, "PICS product info send failed (attempt %d); retrying in %lu ms.", attempt + 1,
                 kRetryDelayMs);
    if (!workers::Submit([attempt] {
            Sleep(kRetryDelayMs);
            Flush(attempt + 1);
        })) {
        AC_LOG_DEBUG(kModule, "PICS product info retry skipped (workers shut down).");
    }
}

void ScheduleOnce() {
    if (s_flushPending.exchange(true, std::memory_order_acq_rel))
        return;
    ScheduleFlush(0);
}

} // namespace

void Request(steam::AppId appId) {
    if (appId == 0 || !s_queue.Enqueue(appId))
        return;
    ScheduleOnce();
}

void RequestAll(const std::vector<steam::AppId>& appIds) {
    bool queued = false;
    for (const steam::AppId appId : appIds) {
        if (appId != 0 && s_queue.Enqueue(appId))
            queued = true;
    }
    if (queued)
        ScheduleOnce();
}

} // namespace ac::hooks::productinfo
