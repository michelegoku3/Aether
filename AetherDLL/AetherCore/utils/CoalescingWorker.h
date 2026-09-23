#pragma once

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <functional>
#include <mutex>
#include <thread>
#include <utility>

namespace ac::utils {
// Private service infrastructure, not domain state. One lifecycle owner calls
// Start/Stop off loader lock; any thread may Request. Callbacks must not throw.
class CoalescingWorker {
public:
    using Callback = std::function<void(std::uint64_t)>;
    ~CoalescingWorker() { Stop(); }
    void Start(Callback callback) {
        if (thread_.joinable()) return;
        stop_.store(false);
        thread_ = std::thread([this, callback = std::move(callback)] {
            for (;;) {
                std::unique_lock lock(mutex_);
                cv_.wait_for(lock, std::chrono::milliseconds(100), [this] { return stop_.load(); });
                lock.unlock();
                if (const auto count = pending_.exchange(0)) callback(count);
                if (stop_.load()) break;
            }
        });
    }
    // No disk I/O, callback, allocation or mutex acquisition on the producer.
    void Request() noexcept { if (!stop_.load()) pending_.fetch_add(1); }
    void Stop() {
        { std::lock_guard lock(mutex_); stop_.store(true); }
        cv_.notify_one();
        if (thread_.joinable()) thread_.join();
    }
private:
    std::atomic<std::uint64_t> pending_{0};
    std::atomic<bool> stop_{false};
    std::mutex mutex_;
    std::condition_variable cv_;
    std::thread thread_;
};
}  // namespace ac::utils
