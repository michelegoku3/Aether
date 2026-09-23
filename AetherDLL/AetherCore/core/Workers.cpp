#include "pch.h"
#include "core/Workers.h"

#include <condition_variable>
#include <deque>
#include <memory>
#include <mutex>
#include <thread>
#include <utility>
#include <vector>

#include "core/Logger.h"

namespace ac::workers {
namespace {

constexpr const char* kModule = "Workers";

// ---- Task queue (one-shot jobs) -------------------------------------------
struct TaskQueue {
    std::mutex mutex;
    std::condition_variable cv;
    std::deque<std::function<void()>> jobs;
    std::thread worker;
    bool stopped = false;
};
TaskQueue s_queue;

void RunJob(const std::function<void()>& job, const char* origin) {
    try {
        job();
    } catch (const std::exception& e) {
        AC_LOG_WARN(kModule, "%s job failed: %s", origin, e.what());
    } catch (...) {
        AC_LOG_WARN(kModule, "%s job failed with unknown exception.", origin);
    }
}

void QueueLoop() {
    for (;;) {
        std::function<void()> job;
        {
            std::unique_lock<std::mutex> lock(s_queue.mutex);
            s_queue.cv.wait(lock, [] { return s_queue.stopped || !s_queue.jobs.empty(); });
            if (s_queue.jobs.empty()) {
                if (s_queue.stopped) return;
                continue;   // spurious wake
            }
            job = std::move(s_queue.jobs.front());
            s_queue.jobs.pop_front();
        }
        RunJob(job, "TaskQueue");
    }
}

void EnsureQueueLocked() {
    if (!s_queue.worker.joinable()) {
        s_queue.worker = std::thread(QueueLoop);
        AC_LOG_INFO(kModule, "Task queue worker started.");
    }
}

// ---- Named worker registry -------------------------------------------------
struct WorkerEntry {
    std::string name;
    std::shared_ptr<std::atomic<bool>> stop;
    std::shared_ptr<std::atomic<bool>> done;
    std::thread thread;
};

std::mutex s_registryMutex;
std::vector<WorkerEntry> s_workers;
std::atomic<bool> s_shutDown{false};
std::atomic<std::uint64_t> s_startedCount{0};

void ReapCompletedLocked() {
    for (auto it = s_workers.begin(); it != s_workers.end();) {
        if (it->done && it->done->load() && it->thread.joinable()) {
            it->thread.join();
            it = s_workers.erase(it);
        } else {
            ++it;
        }
    }
}

}  // namespace

bool Submit(std::function<void()> job) {
    if (!job || s_shutDown.load()) return false;
    {
        std::lock_guard<std::mutex> lock(s_queue.mutex);
        if (s_queue.stopped) return false;
        EnsureQueueLocked();
        s_queue.jobs.push_back(std::move(job));
    }
    s_queue.cv.notify_one();
    return true;
}

bool StartWorker(const std::string& name,
                 std::function<void(std::atomic<bool>&)> body) {
    if (!body || s_shutDown.load()) return false;
    auto stop = std::make_shared<std::atomic<bool>>(false);
    auto done = std::make_shared<std::atomic<bool>>(false);
    {
        std::lock_guard<std::mutex> lock(s_registryMutex);
        ReapCompletedLocked();
        s_workers.push_back(WorkerEntry{name, stop, done, {}});
        WorkerEntry& entry = s_workers.back();
        // La lambda cattura COPIE (name/stop/done): il vector può reallocare
        // a ogni StartWorker, quindi nessun riferimento agli elementi.
        entry.thread = std::thread([name, stop, done, body = std::move(body)] {
            AC_LOG_INFO(kModule, "Worker '%s' started.", name.c_str());
            body(*stop);
            done->store(true);
            AC_LOG_INFO(kModule, "Worker '%s' exited.", name.c_str());
        });
    }
    s_startedCount.fetch_add(1);
    AC_LOG_DEBUG(kModule, "Worker '%s' registered.", name.c_str());
    return true;
}

std::string SummaryText() {
    std::size_t registered = 0;
    std::size_t running = 0;
    {
        std::lock_guard<std::mutex> lock(s_registryMutex);
        registered = s_workers.size();
        for (const auto& w : s_workers) {
            if (w.done && !w.done->load()) ++running;
        }
    }
    std::size_t queued = 0;
    {
        std::lock_guard<std::mutex> lock(s_queue.mutex);
        queued = s_queue.jobs.size();
    }
    return "workers_registered=" + std::to_string(registered) +
           " running=" + std::to_string(running) +
           " queued_jobs=" + std::to_string(queued) +
           " total_started=" + std::to_string(s_startedCount.load());
}

void Shutdown() {
    if (s_shutDown.exchange(true)) return;
    AC_LOG_INFO(kModule, "Shutdown requested (%s).", SummaryText().c_str());

    // 1. Signal every long-lived worker.
    {
        std::lock_guard<std::mutex> lock(s_registryMutex);
        for (auto& w : s_workers) w.stop->store(true);
    }

    // 2. Stop accepting queue jobs, then drain the remaining ones so nothing
    //    scheduled before shutdown is silently lost.
    std::deque<std::function<void()>> remaining;
    {
        std::lock_guard<std::mutex> lock(s_queue.mutex);
        s_queue.stopped = true;
        remaining = std::move(s_queue.jobs);
    }
    s_queue.cv.notify_all();
    if (!remaining.empty()) {
        AC_LOG_INFO(kModule, "Draining %zu pending task-queue job(s).", remaining.size());
    }
    for (const auto& job : remaining) RunJob(job, "TaskQueue(drain)");
    {
        std::thread worker;
        {
            std::lock_guard<std::mutex> lock(s_queue.mutex);
            if (s_queue.worker.joinable()) worker = std::move(s_queue.worker);
        }
        if (worker.joinable()) worker.join();
    }
    AC_LOG_INFO(kModule, "Task queue stopped.");

    // 3. Join every registered worker.
    for (;;) {
        std::thread thread;
        std::string name;
        {
            std::lock_guard<std::mutex> lock(s_registryMutex);
            if (s_workers.empty()) break;
            WorkerEntry entry = std::move(s_workers.back());
            s_workers.pop_back();
            thread = std::move(entry.thread);
            name = std::move(entry.name);
        }
        if (thread.joinable()) thread.join();
        AC_LOG_INFO(kModule, "Worker '%s' joined.", name.c_str());
    }
    AC_LOG_INFO(kModule, "All background workers stopped.");
}

}  // namespace ac::workers
