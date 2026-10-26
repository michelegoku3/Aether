#pragma once

#include <cstdint>
#include <mutex>
#include <unordered_set>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// PICS product-info requests for freshly configured .lua apps.
//
// Steam fills its appinfo cache from the CM's PICS records; the batch job that
// does it for the whole library can sit in Timeout for an entire session when
// the cache is behind. A single-app request is served on the normal path and
// writes the cache entry, which is what makes a new .lua app appear in the
// library. The caller only enqueues: the send happens on a worker, off the
// wire callback that noticed the app.
//
// `RequestQueue` is one-shot per app per process: a directory watch fires on
// every write of the same file, and Steam must not receive the same request
// again for each one. A failed send re-queues through `Requeue`, which bypasses
// the one-shot rule for the retry.
// ---------------------------------------------------------------------------
namespace ac::hooks::productinfo {

class RequestQueue {
  public:
    /// True when this process has not seen `appId` before.
    bool Enqueue(steam::AppId appId) {
        std::lock_guard<std::mutex> lock(mutex_);
        if (!seen_.insert(appId).second)
            return false;
        pending_.push_back(appId);
        return true;
    }

    /// Re-queues an app whose send failed (the one-shot rule is not consulted).
    void Requeue(steam::AppId appId) {
        std::lock_guard<std::mutex> lock(mutex_);
        pending_.push_back(appId);
    }

    /// Moves the pending apps into `out` and clears the queue.
    void Drain(std::vector<steam::AppId>& out) {
        std::lock_guard<std::mutex> lock(mutex_);
        out.insert(out.end(), pending_.begin(), pending_.end());
        pending_.clear();
    }

  private:
    std::mutex mutex_;
    std::unordered_set<steam::AppId> seen_;
    std::vector<steam::AppId> pending_;
};

/// Asks the CM for one app's PICS record, once per process.
void Request(steam::AppId appId);

/// Same for every id, skipping the ones already requested.
void RequestAll(const std::vector<steam::AppId>& appIds);

} // namespace ac::hooks::productinfo
