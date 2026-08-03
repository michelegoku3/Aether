#pragma once

#include <atomic>
#include <chrono>
#include <cstddef>
#include <optional>
#include <shared_mutex>
#include <unordered_map>

// ---------------------------------------------------------------------------
// Generic TTL cache with LRU eviction and negative caching support.
//
// Design principles:
//   - Thread-safe: shared_mutex for readers-writer locking
//   - TTL-based expiration: entries expire after a configurable duration
//   - LRU eviction: when capacity is reached, least-recently-used entries are removed
//   - Negative caching: special "empty" values (V{}) can be cached to avoid repeated lookups
//   - Observable: hit/miss/eviction counters exposed for diagnostics
//
// Usage:
//   TtlCache<AppId, uint64_t> cache(512, std::chrono::hours(24));
//   if (auto value = cache.Get(appId)) {
//       // cache hit (value may be "empty" for negative cache)
//   } else {
//       // cache miss: compute value and call Put() or PutNegative()
//   }
//
// Thread safety:
//   - Get() uses shared lock (multiple concurrent readers)
//   - Put()/PutNegative()/EvictExpired() use unique lock (exclusive writer)
//   - Counters use atomic operations for lock-free reads
// ---------------------------------------------------------------------------
namespace ac::utils {

template <typename K, typename V>
class TtlCache {
public:
    using Clock = std::chrono::steady_clock;
    using TimePoint = Clock::time_point;
    using Duration = std::chrono::seconds;

    struct Entry {
        V value;
        TimePoint expiresAt;
        TimePoint lastAccess;
    };

    // Constructs a cache with the given capacity (max entries) and TTL (time-to-live).
    TtlCache(std::size_t maxEntries, Duration ttl)
        : maxEntries_(maxEntries), ttl_(ttl) {}

    // Retrieves a value from the cache. Returns nullopt on miss.
    // Updates lastAccess timestamp for LRU tracking.
    std::optional<V> Get(const K& key) {
        std::shared_lock<std::shared_mutex> lock(mutex_);
        auto it = entries_.find(key);
        if (it == entries_.end()) {
            ++missCount_;
            return std::nullopt;
        }

        // Check expiration
        if (Clock::now() >= it->second.expiresAt) {
            lock.unlock();
            // Upgrade to unique lock for eviction
            std::unique_lock<std::shared_mutex> writeLock(mutex_);
            // Re-check: another thread may have already evicted it
            it = entries_.find(key);
            if (it != entries_.end() && Clock::now() >= it->second.expiresAt) {
                entries_.erase(it);
                ++evictionCount_;
            }
            ++missCount_;
            return std::nullopt;
        }

        // Update lastAccess for LRU (requires unique lock)
        lock.unlock();
        {
            std::unique_lock<std::shared_mutex> writeLock(mutex_);
            it = entries_.find(key);
            if (it != entries_.end()) {
                it->second.lastAccess = Clock::now();
            }
        }

        ++hitCount_;
        return it->second.value;
    }

    // Inserts or updates a value in the cache.
    void Put(const K& key, const V& value) {
        std::unique_lock<std::shared_mutex> lock(mutex_);
        const auto now = Clock::now();

        // Evict expired entries first (opportunistic cleanup)
        EvictExpiredLocked();

        // If at capacity, evict LRU entry (unless we're updating an existing key)
        if (entries_.size() >= maxEntries_ && entries_.find(key) == entries_.end()) {
            EvictLruLocked();
        }

        entries_[key] = Entry{value, now + ttl_, now};
    }

    // Inserts a "negative" cache entry (empty value) to avoid repeated lookups.
    void PutNegative(const K& key) {
        Put(key, V{});
    }

    // Removes all expired entries. Can be called periodically or on-demand.
    void EvictExpired() {
        std::unique_lock<std::shared_mutex> lock(mutex_);
        EvictExpiredLocked();
    }

    // Returns the current number of entries in the cache.
    std::size_t Size() const {
        std::shared_lock<std::shared_mutex> lock(mutex_);
        return entries_.size();
    }

    // Returns the number of negative entries (value == V{}).
    std::size_t NegativeCount() const {
        std::shared_lock<std::shared_mutex> lock(mutex_);
        std::size_t count = 0;
        for (const auto& [key, entry] : entries_) {
            if (entry.value == V{}) ++count;
        }
        return count;
    }

    // Returns the number of cache hits.
    std::size_t HitCount() const { return hitCount_.load(std::memory_order_relaxed); }

    // Returns the number of cache misses.
    std::size_t MissCount() const { return missCount_.load(std::memory_order_relaxed); }

    // Returns the number of evictions (expired or LRU).
    std::size_t EvictionCount() const { return evictionCount_.load(std::memory_order_relaxed); }

    // Returns the configured capacity.
    std::size_t MaxEntries() const { return maxEntries_; }

    // Returns the configured TTL.
    Duration Ttl() const { return ttl_; }

private:
    std::unordered_map<K, Entry> entries_;
    std::size_t maxEntries_;
    Duration ttl_;

    mutable std::shared_mutex mutex_;
    std::atomic<std::size_t> hitCount_{0};
    std::atomic<std::size_t> missCount_{0};
    std::atomic<std::size_t> evictionCount_{0};

    // Removes all expired entries. Caller must hold unique lock.
    void EvictExpiredLocked() {
        const auto now = Clock::now();
        std::size_t evicted = 0;
        for (auto it = entries_.begin(); it != entries_.end();) {
            if (now >= it->second.expiresAt) {
                it = entries_.erase(it);
                ++evicted;
            } else {
                ++it;
            }
        }
        evictionCount_.fetch_add(evicted, std::memory_order_relaxed);
    }

    // Removes the least-recently-used entry. Caller must hold unique lock.
    void EvictLruLocked() {
        if (entries_.empty()) return;

        auto lruIt = entries_.begin();
        for (auto it = entries_.begin(); it != entries_.end(); ++it) {
            if (it->second.lastAccess < lruIt->second.lastAccess) {
                lruIt = it;
            }
        }
        entries_.erase(lruIt);
        ++evictionCount_;
    }
};

}  // namespace ac::utils
