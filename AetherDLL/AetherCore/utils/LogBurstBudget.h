#pragma once
#include <chrono>
#include <cstdint>
#include <mutex>
namespace ac::logutil {
// Fixed-memory budget for verbose detail attempts, not state/error events.
// Suppressed count is reported on the next attempt after the window expires.
class LogBurstBudget {
public:
    using Clock = std::chrono::steady_clock;
    struct Decision { bool emit; std::uint64_t suppressed; };
    explicit LogBurstBudget(unsigned limit = 20,
                            Clock::duration window = std::chrono::seconds(10))
        : limit_(limit), window_(window) {}
    Decision Admit(Clock::time_point now = Clock::now()) {
        std::lock_guard lock(mutex_);
        std::uint64_t previous = 0;
        if (!started_ || now - start_ >= window_) {
            previous = suppressed_;
            start_ = now;
            started_ = true;
            used_ = 0;
            suppressed_ = 0;
        }
        if (used_ < limit_) { ++used_; return {true, previous}; }
        ++suppressed_;
        return {false, previous};
    }
private:
    std::mutex mutex_;
    const unsigned limit_;
    const Clock::duration window_;
    Clock::time_point start_{};
    bool started_ = false;
    unsigned used_ = 0;
    std::uint64_t suppressed_ = 0;
};
}  // namespace ac::logutil
