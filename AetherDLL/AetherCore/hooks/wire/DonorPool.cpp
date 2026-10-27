#include "pch.h"
#include "hooks/wire/DonorPool.h"

#include <algorithm>
#include <deque>
#include <fstream>
#include <iterator>
#include <mutex>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/WinFile.h"

namespace ac::hooks::DonorPool {
namespace {

constexpr const char* kModule = "Wire.Achievement";

// LumaCore's default/primary stat SteamID (kDefaultStatSteamId) is the LAST
// entry of the compiled pool: 76561198028121353 == slot 14. LumaCore's
// DefaultPoolIndex() tries this ID FIRST for a new app. Starting at slot 14
// matches that: a game owned only by that donor responds on the first attempt
// instead of cycling indices 0,1,2,... and giving up before reaching 14.

constexpr auto kAttemptWindow = std::chrono::seconds(15);
constexpr std::size_t kAttemptCap = 24;

constexpr auto kPendingWindow = std::chrono::seconds(30);

constexpr std::string_view kLearningHeader = "aether-donors 2";
constexpr std::size_t kPoolHexChars = kMaxPoolCount / 4; // bitset -> hex
constexpr std::size_t kFingerprintHexChars = 16;

// Central-state accessor: all runtime data lives in
// g_state.achievements.donor (real AetherCoreState, or the test seam's
// identical struct). Per-group locks in SharedState preserve the existing
// locking discipline.
SharedState& S() { return g_state.achievements.donor; }

uint64_t Fnv1a64(const std::uint8_t* data, std::size_t size, std::uint64_t h = 1469598103934665603ULL) {
    for (std::size_t i = 0; i < size; ++i) {
        h ^= data[i];
        h *= 1099511628211ULL;
    }
    return h;
}

// Fingerprint over the active donor list, in order: indices are only meaningful
// for the exact pool that produced them.
std::uint64_t FingerprintLocked(const SharedState& s) {
    std::uint64_t h = 1469598103934665603ULL;
    for (const std::uint64_t id : s.donors) {
        std::uint8_t bytes[8] = {};
        for (std::size_t i = 0; i < 8; ++i)
            bytes[i] = static_cast<std::uint8_t>((id >> (i * 8)) & 0xFFULL);
        h = Fnv1a64(bytes, sizeof(bytes), h);
    }
    return h;
}

std::string Hex(uint64_t value) {
    static constexpr char kDigits[] = "0123456789abcdef";
    std::string out(kFingerprintHexChars, '0');
    for (int i = 15; i >= 0; --i) {
        out[static_cast<std::size_t>(i)] = kDigits[value & 0xFu];
        value >>= 4;
    }
    return out;
}

int HexDigit(char c) {
    if (c >= '0' && c <= '9')
        return c - '0';
    if (c >= 'a' && c <= 'f')
        return c - 'a' + 10;
    if (c >= 'A' && c <= 'F')
        return c - 'A' + 10;
    return -1;
}

// Tried mask as fixed-width hex, bit 0 in the last character.
std::string HexBits(const std::bitset<kMaxPoolCount>& bits) {
    static constexpr char kDigits[] = "0123456789abcdef";
    std::string out(kPoolHexChars, '0');
    for (std::size_t n = 0; n < kPoolHexChars; ++n) {
        unsigned value = 0;
        for (std::size_t b = 0; b < 4; ++b) {
            const std::size_t bit = (kPoolHexChars - 1 - n) * 4 + b;
            if (bit < kMaxPoolCount && bits.test(bit))
                value |= 1u << b;
        }
        out[n] = kDigits[value & 0xFu];
    }
    return out;
}

bool ParseHexBits(std::string_view text, std::bitset<kMaxPoolCount>& out) {
    if (text.size() != kPoolHexChars)
        return false;
    out.reset();
    for (std::size_t n = 0; n < text.size(); ++n) {
        const int digit = HexDigit(text[n]);
        if (digit < 0)
            return false;
        for (std::size_t b = 0; b < 4; ++b) {
            if ((digit & (1 << b)) != 0)
                out.set((text.size() - 1 - n) * 4 + b);
        }
    }
    return true;
}

bool ParseHex64(std::string_view text, std::uint64_t& out) {
    if (text.size() != 16)
        return false;
    std::uint64_t value = 0;
    for (const char c : text) {
        const int digit = HexDigit(c);
        if (digit < 0)
            return false;
        value = (value << 4) | static_cast<std::uint64_t>(digit);
    }
    out = value;
    return true;
}

bool ParseUnsigned(std::string_view text, std::uint64_t& out) {
    if (text.empty() || text.size() > 20)
        return false;
    std::uint64_t value = 0;
    for (char c : text) {
        if (c < '0' || c > '9')
            return false;
        value = value * 10 + static_cast<std::uint64_t>(c - '0');
    }
    out = value;
    return true;
}

void PruneAttemptsLocked(Clock::time_point now) {
    SharedState& s = S();
    std::erase_if(s.jobIdToAttempt, [&now](const auto& e) { return now - e.second.seen > std::chrono::seconds(30); });
    for (auto it = s.recentAttempts.begin(); it != s.recentAttempts.end();) {
        if (now - it->seen > kAttemptWindow)
            it = s.recentAttempts.erase(it);
        else
            ++it;
    }
    while (s.recentAttempts.size() > kAttemptCap)
        s.recentAttempts.pop_front();
}

// Next slot the walk should probe: the first untried slot at/after `next`.
// `count` once the walk is over (a donor with data was found, or every slot
// already answered without data).
std::size_t NextUntriedLocked(const PoolEntry& e, std::size_t count) {
    if (count == 0 || e.hasPreferred)
        return count;
    for (std::size_t i = 0; i < count; ++i) {
        const std::size_t index = (e.next + i) % count;
        if (!e.tried.test(index))
            return index;
    }
    return count;
}

} // namespace

std::size_t PoolCount() {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    return s.donors.size();
}

std::uint64_t PoolFingerprint() {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    return FingerprintLocked(s);
}

std::uint64_t DonorSteamId(std::size_t index) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    if (s.donors.empty())
        return 0;
    return s.donors[index % s.donors.size()];
}

std::size_t PickIndex(steam::AppId appId) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    const std::size_t count = s.donors.size();
    if (count == 0)
        return 0;
    auto& e = s.pool[appId];
    if (e.hasPreferred && e.preferred < count)
        return e.preferred;
    const std::size_t untried = NextUntriedLocked(e, count);
    // Every slot tried: hand out a valid donor anyway (the client keeps asking);
    // the probe chain treats the pool as exhausted.
    return untried < count ? untried : e.next % count;
}

std::size_t NextUntried(steam::AppId appId) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    return NextUntriedLocked(s.pool[appId], s.donors.size());
}

void NoteAttemptResult(steam::AppId appId, std::size_t index, bool okWithData) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    const std::size_t count = s.donors.size();
    if (count == 0)
        return;
    auto& e = s.pool[appId];
    if (index < count)
        e.tried.set(index);
    if (okWithData) {
        e.preferred = index;
        e.hasPreferred = true;
        e.next = index;
        e.exhaustedAt = 0;
        AC_LOG_INFO_ONCE(kModule, "Pool AppID %u: preferred index %zu (donor %llu has data).", appId, index,
                         s.donors[index % count]);
        return;
    }
    if (e.hasPreferred && e.preferred == index) {
        e.hasPreferred = false;
        e.preferred = 0;
    }
    e.next = (index + 1) % count;
    AC_LOG_DEBUG(kModule, "Pool AppID %u: advancing index %zu -> %zu (donor has no data).", appId, index, e.next);
}

std::string SerializeLearning() {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);

    std::vector<std::pair<steam::AppId, PoolEntry>> rows(s.pool.begin(), s.pool.end());
    std::sort(rows.begin(), rows.end(), [](const auto& a, const auto& b) { return a.first < b.first; });

    std::string out(kLearningHeader);
    out += "\npool=";
    out += Hex(FingerprintLocked(s));
    out += '\n';
    for (const auto& [appId, entry] : rows) {
        if (!entry.hasPreferred && entry.tried.none() && entry.exhaustedAt == 0)
            continue;
        out += std::to_string(appId);
        out += " donor=";
        out += entry.hasPreferred ? std::to_string(entry.preferred) : "-";
        out += " tried=";
        out += HexBits(entry.tried);
        if (entry.exhaustedAt != 0) {
            out += " exhausted=";
            out += std::to_string(entry.exhaustedAt);
        }
        out += '\n';
    }
    return out;
}

void NotePoolExhausted(steam::AppId appId, std::uint64_t unixSeconds) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    s.pool[appId].exhaustedAt = unixSeconds;
}

std::uint64_t PoolExhaustedAt(steam::AppId appId) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    return s.pool[appId].exhaustedAt;
}

void ResetWalk(steam::AppId appId) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    PoolEntry& e = s.pool[appId];
    e.tried.reset();
    e.exhaustedAt = 0;
    e.next = kDefaultPoolIndex < s.donors.size() ? kDefaultPoolIndex : 0;
}

bool SaveLearning(const std::string& path) {
    if (path.empty())
        return false;
    return winfile::WriteTextAtomically(path, SerializeLearning());
}

void LoadLearningFile(const std::string& path) {
    if (path.empty())
        return;
    std::ifstream in(winfile::ToPath(path), std::ios::binary);
    if (!in)
        return; // first run: nothing to resume
    std::string text((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    LoadLearning(text, /*replace=*/true);
}

void LoadLearning(const std::string& text, bool replace) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.poolMutex);
    if (replace)
        s.pool.clear();

    std::size_t pos = 0;
    bool headerSeen = false;
    bool poolMatches = false;
    while (pos < text.size()) {
        const std::size_t end = text.find('\n', pos);
        std::string_view line(text.data() + pos, (end == std::string::npos ? text.size() : end) - pos);
        pos = end == std::string::npos ? text.size() : end + 1;
        // The snapshot is a user-editable file: accept Windows line endings.
        if (!line.empty() && line.back() == '\r')
            line.remove_suffix(1);

        if (!headerSeen) {
            if (line != kLearningHeader) {
                AC_LOG_DEBUG(kModule, "Learning: unknown snapshot header, ignored.");
                return;
            }
            headerSeen = true;
            continue;
        }
        // The fingerprint proves the snapshot belongs to THIS pool: indices from
        // another list would point at different donors.
        if (line.starts_with("pool=")) {
            std::uint64_t stored = 0;
            poolMatches = ParseHex64(line.substr(5), stored) && stored == FingerprintLocked(s);
            if (!poolMatches) {
                AC_LOG_INFO(kModule, "Donor pool changed since the learning file was written: learning discarded.");
                s.pool.clear();
                return;
            }
            continue;
        }
        if (!poolMatches)
            continue; // no fingerprint line yet: refuse to apply indices
        if (line.empty())
            continue;

        // "<appid> donor=<index>| tried=<hex>[ exhausted=<secs>]"
        const std::size_t firstSpace = line.find(' ');
        if (firstSpace == std::string_view::npos)
            continue;
        std::uint64_t appIdValue = 0;
        if (!ParseUnsigned(line.substr(0, firstSpace), appIdValue))
            continue;

        PoolEntry entry;
        bool parsedAny = false;
        std::size_t fieldPos = firstSpace + 1;
        while (fieldPos < line.size()) {
            const std::size_t fieldEnd = line.find(' ', fieldPos);
            const std::string_view field =
                line.substr(fieldPos, (fieldEnd == std::string_view::npos ? line.size() : fieldEnd) - fieldPos);
            fieldPos = fieldEnd == std::string_view::npos ? line.size() : fieldEnd + 1;

            if (field.starts_with("donor=")) {
                const std::string_view value = field.substr(6);
                std::uint64_t index = 0;
                if (ParseUnsigned(value, index) && index < s.donors.size()) {
                    entry.preferred = static_cast<std::size_t>(index);
                    entry.hasPreferred = true;
                    entry.next = entry.preferred;
                    parsedAny = true;
                }
            } else if (field.starts_with("tried=")) {
                if (ParseHexBits(field.substr(6), entry.tried))
                    parsedAny = true;
            } else if (field.starts_with("exhausted=")) {
                std::uint64_t stamp = 0;
                if (ParseUnsigned(field.substr(10), stamp)) {
                    entry.exhaustedAt = stamp;
                    parsedAny = true;
                }
            }
        }
        if (parsedAny)
            s.pool[static_cast<steam::AppId>(appIdValue)] = entry;
    }
}

void RecordAttempt(StatAttempt a, bool hasJobId, std::uint64_t jobId) {
    auto now = Clock::now();
    a.seen = now;
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.attemptMutex);
    PruneAttemptsLocked(now);
    a.sequence = s.nextSequence++;
    if (a.probe) {
        // Probe requests are ours: they correlate by jobid only. Leaving them
        // out of the fallback queue keeps client responses from being attributed
        // to a probe (and vice versa).
        if (hasJobId)
            s.jobIdToAttempt[jobId] = a;
        return;
    }
    if (hasJobId)
        s.jobIdToAttempt[jobId] = a;
    s.recentAttempts.push_back(a);
}

Correl ResolveAttempt(bool hasJobIdTarget, std::uint64_t jobIdTarget, StatAttempt& out) {
    auto now = Clock::now();
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.attemptMutex);
    PruneAttemptsLocked(now);

    if (hasJobIdTarget) {
        auto it = s.jobIdToAttempt.find(jobIdTarget);
        if (it != s.jobIdToAttempt.end()) {
            out = it->second;
            s.jobIdToAttempt.erase(it);
            // Successful correlation: also drop the attempt from the fallback
            // queue, otherwise it lingers as a "ghost" for kAttemptWindow and
            // makes the next response's fallback ambiguous (n>1), which would
            // then reach the game UNREWRITTEN (with the donor's stats).
            for (auto itDq = s.recentAttempts.begin(); itDq != s.recentAttempts.end();) {
                if (itDq->sequence == out.sequence)
                    itDq = s.recentAttempts.erase(itDq);
                else
                    ++itDq;
            }
            AC_LOG_DEBUG(kModule, "Response correlation via jobid_target %llu -> attempt AppID %u (pool %zu).",
                         static_cast<unsigned long long>(jobIdTarget), out.appId, out.poolIndex);
            return Correl::Resolved;
        }
    }
    // Fallback: a single recent in-flight request for this pipe.
    StatAttempt cand;
    std::size_t n = 0;
    for (const auto& a : s.recentAttempts) {
        if (now - a.seen <= kAttemptWindow) {
            cand = a;
            ++n;
        }
    }
    if (n == 1) {
        out = cand;
        for (auto it = s.recentAttempts.begin(); it != s.recentAttempts.end();) {
            if (it->sequence == cand.sequence)
                it = s.recentAttempts.erase(it);
            else
                ++it;
        }
        AC_LOG_DEBUG(kModule, "Response correlation without jobid: single recent attempt -> AppID %u (pool %zu).",
                     out.appId, out.poolIndex);
        return Correl::Resolved;
    }
    if (n > 1) {
        AC_LOG_WARN(kModule,
                    "Ambiguous correlation: %zu overlapping spoofed requests in flight. The response will "
                    "be stripped of the donor payload for safety (no leak to the game).",
                    n);
        return Correl::Ambiguous;
    }
    return Correl::NoMatch;
}

void RecordPendingClientStats(steam::AppId appId, StatAttempt attempt) {
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.pendingMutex);
    auto now = Clock::now();
    std::erase_if(s.pendingClientStats, [&now](const auto& e) { return now - e.second.seen > kPendingWindow; });
    s.pendingClientStats[appId] = attempt;
}

bool TakePendingClientStats(steam::AppId appId, StatAttempt& out) {
    auto now = Clock::now();
    SharedState& s = S();
    std::lock_guard<std::mutex> lock(s.pendingMutex);
    std::erase_if(s.pendingClientStats, [&now](const auto& e) { return now - e.second.seen > kPendingWindow; });
    auto it = s.pendingClientStats.find(appId);
    if (it == s.pendingClientStats.end())
        return false;
    out = it->second;
    s.pendingClientStats.erase(it);
    return true;
}

} // namespace ac::hooks::DonorPool
