#include "pch.h"
#include "network/HubcapQuota.h"

#include <chrono>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <optional>
#include <string>
#include <thread>

#include "core/Constants.h"
#include "core/Logger.h"
#include "hooks/wire/BackupIo.h"
#include "utils/JsonStringField.h"

namespace ac::hubcapquota {
namespace {

constexpr const char* kModule = "HubcapQuota";
// Shared values live in core/Constants.h (contract twins in
// docs/shared_contracts.md §4); aliases keep call sites short.
constexpr std::uint64_t kMaxGamePerDay = constants::kHubcapMaxGamePerDay;
constexpr int kLockStaleAfterSec = constants::kHubcapLockStaleAfterSec;
constexpr int kLockTimeoutSec = constants::kHubcapLockTimeoutSec;
constexpr auto kLockRetryInterval = std::chrono::milliseconds(constants::kHubcapLockRetryMs);
constexpr DWORD kDeleteOnClose = 0x04000000; // FILE_FLAG_DELETE_ON_CLOSE

std::string QuotaPath() {
    const std::string dataDir = backup::io::CachedDeskDataDir();
    if (dataDir.empty())
        return std::string();
    return (std::filesystem::path(dataDir) / "state" / "hubcap_generation_quota.json").string();
}

std::string LockPathFor(const std::string& quotaPath) {
    std::filesystem::path lock(quotaPath);
    lock.replace_extension(".lock");
    return lock.string();
}

// Fixed UTC-5 calendar day ("YYYY-MM-DD") — the same contract as AetherDesk's
// est_day_from_unix: the provider resets the budget at midnight EST, never at
// the host's local midnight, so DST must never shift the boundary.
std::string CurrentEstDay() {
    const std::int64_t now = static_cast<std::int64_t>(std::time(nullptr));
    const std::int64_t days = (now - 5LL * 3600) / 86'400;
    // Howard Hinnant's civil_from_days, with a Unix epoch offset (twin of the
    // Rust helper — keep the two in sync).
    const std::int64_t z = days + 719'468;
    const std::int64_t era = (z >= 0 ? z : z - 146'096) / 146'097;
    const std::int64_t doe = z - era * 146'097;
    const std::int64_t yoe = (doe - doe / 1'460 + doe / 36'524 - doe / 146'096) / 365;
    const std::int64_t y = yoe + era * 400;
    const std::int64_t doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    const std::int64_t mp = (5 * doy + 2) / 153;
    const std::int64_t d = doy - (153 * mp + 2) / 5 + 1;
    const std::int64_t m = mp + (mp < 10 ? 3 : -9);
    const std::int64_t year = y + (m <= 2 ? 1 : 0);
    char day[16] = {};
    // Fixed-width date (10 chars into 16): truncation cannot occur.
    (void)std::snprintf(day, sizeof(day), "%04lld-%02lld-%02lld", static_cast<long long>(year),
                        static_cast<long long>(m), static_cast<long long>(d));
    return day;
}

struct QuotaState {
    std::string day;
    std::uint64_t gameUsed = 0;
    std::uint64_t workshopUsed = 0;
};

// Reads the shared quota, resetting it when the fixed-EST day rolled over.
//
// Fail-closed by design: a MISSING file is a fresh install (start at zero,
// allowed), but a PRESENT file that cannot be parsed is never silently
// reset to zero — that would restore the full daily budget to whoever
// corrupts one JSON file. Callers refuse the generation and keep a
// timestamped .corrupt copy plus a diagnostic for status.json.
std::optional<QuotaState> ReadQuota(const std::string& path) {
    const std::string today = CurrentEstDay();

    std::error_code ec;
    if (!std::filesystem::exists(path, ec) || ec) {
        QuotaState fresh;
        fresh.day = today;
        return fresh;
    }

    std::ifstream input(path, std::ios::binary);
    if (!input)
        return std::nullopt;

    const std::string body((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
    QuotaState state;
    state.day = today;
    bool parsed = false;
    std::string day;
    if (ac::jsonutil::PullStringField(body, "day", day)) {
        state.day = day;
        parsed = true;
    }
    std::uint64_t value = 0;
    if (ac::jsonutil::PullUIntField(body, "game_used", value)) {
        state.gameUsed = value;
        parsed = true;
    }
    if (ac::jsonutil::PullUIntField(body, "workshop_used", value)) {
        state.workshopUsed = value;
        parsed = true;
    }
    if (!parsed)
        return std::nullopt;
    if (state.day != today) {
        state = QuotaState{};
        state.day = today;
    }
    return state;
}

// Preserves the unreadable file for forensics before the caller refuses.
// Best-effort: a failed copy must not change the refusal outcome.
void PreserveCorruptCopy(const std::string& path) {
    try {
        const std::string backup = path + ".corrupt-" + std::to_string(std::time(nullptr));
        std::error_code ec;
        std::filesystem::copy_file(path, backup, std::filesystem::copy_options::none, ec);
        if (!ec)
            AC_LOG_WARN(kModule, "Preserved unreadable quota file as %s.", backup.c_str());
        // Forensics copy is best-effort by design; failures must not change
        // the refusal outcome above.
        // NOLINTNEXTLINE(bugprone-empty-catch)
    } catch (...) {
    }
}

bool WriteQuota(const std::string& path, const QuotaState& state) {
    const std::filesystem::path target(path);
    const std::string directory = target.parent_path().string();
    if (!directory.empty() && !CreateDirectoryA(directory.c_str(), nullptr) && GetLastError() != ERROR_ALREADY_EXISTS) {
        return false;
    }
    char body[96] = {};
    const int written = std::snprintf(body, sizeof(body), "{\"day\":\"%s\",\"game_used\":%llu,\"workshop_used\":%llu}",
                                      state.day.c_str(), static_cast<unsigned long long>(state.gameUsed),
                                      static_cast<unsigned long long>(state.workshopUsed));
    if (written <= 0 || written >= static_cast<int>(sizeof(body)))
        return false;
    const std::string temporary = path + ".tmp";
    {
        std::ofstream output(temporary, std::ios::binary | std::ios::trunc);
        if (!output)
            return false;
        output.write(body, static_cast<std::streamsize>(std::strlen(body)));
        if (!output)
            return false;
    }
    return backup::io::AtomicReplace(temporary, path);
}

// Exclusive create-new lock with delete-on-close semantics: closing the handle
// — normally or after a crash — removes the file, so the lock can never
// outlive its owner; a lock that somehow survives is broken after
// kLockStaleAfterSec. The critical section it protects is a tiny
// read-modify-write, so staleness can only mean a dead holder.
class ScopedQuotaLock {
  public:
    ScopedQuotaLock() = default;
    ScopedQuotaLock(const ScopedQuotaLock&) = delete;
    ScopedQuotaLock& operator=(const ScopedQuotaLock&) = delete;

    ~ScopedQuotaLock() {
        if (handle_ != INVALID_HANDLE_VALUE)
            CloseHandle(handle_); // OS deletes the file
    }

    bool Acquire(const std::string& path) {
        const std::string directory = std::filesystem::path(path).parent_path().string();
        if (!directory.empty()) {
            // The state directory may not exist on a fresh install yet.
            CreateDirectoryA(directory.c_str(), nullptr);
        }
        const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(kLockTimeoutSec);
        for (;;) {
            HANDLE handle = CreateFileA(path.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                                        FILE_ATTRIBUTE_NORMAL | kDeleteOnClose, nullptr);
            if (handle != INVALID_HANDLE_VALUE) {
                handle_ = handle;
                return true;
            }
            const DWORD error = GetLastError();
            if (error != ERROR_FILE_EXISTS && error != ERROR_ACCESS_DENIED) {
                AC_LOG_ERROR(kModule, "Quota lock create failed (error=%lu).", error);
                return false;
            }
            std::error_code ec;
            const auto modified = std::filesystem::last_write_time(path, ec);
            if (!ec) {
                const auto age = std::filesystem::file_time_type::clock::now() - modified;
                if (age > std::chrono::seconds(kLockStaleAfterSec) && DeleteFileA(path.c_str())) {
                    AC_LOG_WARN(kModule, "Broke stale Hubcap quota lock %s.", path.c_str());
                    continue;
                }
            }
            if (std::chrono::steady_clock::now() >= deadline) {
                AC_LOG_ERROR(kModule, "Hubcap quota lock stayed busy for %d s: %s.", kLockTimeoutSec, path.c_str());
                return false;
            }
            std::this_thread::sleep_for(kLockRetryInterval);
        }
    }

  private:
    HANDLE handle_ = INVALID_HANDLE_VALUE;
};

} // namespace

bool TryReserveGameGeneration() {
    const std::string quotaPath = QuotaPath();
    if (quotaPath.empty()) {
        // No AetherData: nothing to account against locally. Generation is
        // still gated by the provider key and by the server's own limits.
        AC_LOG_DEBUG(kModule, "Shared quota unavailable (AetherData not configured).");
        return true;
    }

    ScopedQuotaLock lock;
    if (!lock.Acquire(LockPathFor(quotaPath)))
        return false;

    auto quotaOpt = ReadQuota(quotaPath);
    if (!quotaOpt) {
        // Present but unreadable: fail closed (never reset the budget) and
        // keep evidence. Delete or repair the file to resume generating.
        AC_LOG_WARN(kModule, "Shared quota file unreadable, refusing generation (see %s.corrupt-*).",
                    quotaPath.c_str());
        PreserveCorruptCopy(quotaPath);
        diag::Record("quota_corrupt", quotaPath);
        return false;
    }
    QuotaState quota = *quotaOpt;
    if (quota.gameUsed >= kMaxGamePerDay) {
        AC_LOG_WARN(kModule, "Shared game-manifest budget exhausted (%llu/%llu; resets at midnight EST).",
                    static_cast<unsigned long long>(quota.gameUsed), static_cast<unsigned long long>(kMaxGamePerDay));
        return false;
    }
    quota.gameUsed += 1;
    if (!WriteQuota(quotaPath, quota)) {
        AC_LOG_WARN(kModule, "Could not persist the shared quota; blocking generation.");
        return false;
    }
    AC_LOG_INFO(kModule, "Shared game-manifest budget reserved (%llu/%llu used today).",
                static_cast<unsigned long long>(quota.gameUsed), static_cast<unsigned long long>(kMaxGamePerDay));
    return true;
}

void ReleaseGameGeneration() {
    const std::string quotaPath = QuotaPath();
    if (quotaPath.empty())
        return;

    ScopedQuotaLock lock;
    if (!lock.Acquire(LockPathFor(quotaPath)))
        return; // already logged

    auto quotaOpt = ReadQuota(quotaPath);
    if (!quotaOpt) {
        AC_LOG_WARN(kModule, "Shared quota file unreadable, skipping release (see %s.corrupt-*).", quotaPath.c_str());
        PreserveCorruptCopy(quotaPath);
        diag::Record("quota_corrupt", quotaPath);
        return;
    }
    QuotaState quota = *quotaOpt;
    if (quota.gameUsed > 0)
        quota.gameUsed -= 1;
    if (!WriteQuota(quotaPath, quota)) {
        AC_LOG_WARN(kModule, "Could not persist the shared quota release.");
    } else {
        AC_LOG_INFO(kModule, "Shared game-manifest budget released (%llu used today).",
                    static_cast<unsigned long long>(quota.gameUsed));
    }
}

} // namespace ac::hubcapquota
