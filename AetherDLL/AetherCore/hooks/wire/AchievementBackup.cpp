#include "pch.h"
#include "hooks/wire/AchievementBackup.h"

#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <ctime>
#include <deque>
#include <mutex>
#include <optional>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/PlaytimeMirror.h"
#include "hooks/wire/SteamStatsCache.h"
#include "hooks/wire/UserStatsSnapshot.h"
#include "scripting/LuaData.h"

// ============================================================================
// AchievementBackup — loss-prevention facade (achievements + stats + playtime).
//
// This module is ONLY the async coordinator: job queue + worker thread.
// Real persistence lives in the cohesive modules under hooks/wire/:
//   * io::         AetherData paths, times, atomic writes
//   * snapshot::   achievement/stat JSON store (monotone merge rules)
//   * statscache:: .bin copy with monotone guard + schema interpretation
//   * playtime::   playtime mirror from localconfig.vdf
// Public contract unchanged (see AchievementBackup.h): RecordUnlock /
// RecordStats / TouchSession / BackupAllKnownStatsAtStartup /
// FlushOnShutdown. Enqueue calls are non-blocking (no I/O on the network
// thread). FlushOnShutdown drains and awaits completion; the registry joins.
// Only outside the loader lock.
// ============================================================================

namespace ac::hooks::AchievementBackup {

// Short aliases of the persistence modules (see hooks/wire/).
namespace io = ac::backup::io;
namespace snapshot = ac::backup::snapshot;
namespace statscache = ac::backup::statscache;
namespace playtime = ac::backup::playtime;

namespace {

constexpr const char* kModule = "Wire.Achievement";

// Periodic checkpoint (every 5 minutes): forced re-copy of the .bins for every
// (app, account) touched in session + playtime refresh. The safety net capping
// maximum loss at 5 minutes even when Steam exits without an explicit shutdown
// (the monotone guard protects against regressions).
constexpr auto kCheckpointInterval = std::chrono::minutes(5);
// The end-of-session final copy starts delayed: Steam writes the stats cache
// a few moments AFTER the empty games_played frame.
constexpr auto kSessionEndDelay = std::chrono::seconds(15);

enum class JobType {
    Unlock,      // refresh the JSON snapshot + copy the .bins (forced)
    BinCopy,     // copy only the .bins (e.g. session's first 818/151/5466)
    StatsUpdate, // refresh the JSON snapshot's stats section
    StartupScan, // initial snapshot of every .bin + playtime (once/process)
};

using SteadyClock = std::chrono::steady_clock;

struct BackupJob {
    JobType type = JobType::Unlock;
    steam::AppId appId = 0;
    std::uint32_t accountId = 0;
    std::uint64_t steamId64 = 0;
    std::uint32_t achievementId = 0;
    std::uint32_t unlockTime = 0;
    std::vector<std::pair<std::uint32_t, std::uint32_t>> stats;
    bool force = false;              // skip the copy rate-limit
    SteadyClock::time_point runAt{}; // epoch = run immediately
};

std::mutex g_workerMutex;
std::condition_variable g_workerCv;
std::deque<BackupJob> g_jobs;
bool g_stopping = false;
std::size_t g_failedJobs = 0;       // worker-owned; read only after completion
workers::WorkerCompletion g_worker; // registry owns the thread; flush waits on completion

// Worker-exclusive ownership (no lock needed).
std::unordered_map<steam::AppId, std::unordered_set<std::uint32_t>> g_touched;

// TouchSession per-process dedup: a single BinCopy per app per Steam session
// (the file does not change until the game saves).
std::mutex g_touchMutex;
std::unordered_set<steam::AppId> g_sessionTouched;

// ---------------------------------------------------------------------------
// Job execution (worker thread only: owns g_touched and the disk).
// ---------------------------------------------------------------------------

void ProcessUnlock(const BackupJob& job) {
    const std::string path = snapshot::SnapshotPath(job.appId, job.accountId);
    if (path.empty())
        return; // backup unavailable (no AetherData)

    g_touched[job.appId].insert(job.accountId);

    snapshot::SnapshotData snap = snapshot::Load(path);
    snapshot::MergeUnlock(snap, job.achievementId, job.unlockTime);
    snapshot::SortAll(snap);
    snapshot::Save(path, job.appId, job.accountId, job.steamId64, snap);
    AC_LOG_DEBUG(kModule, "Backup: snapshot %s now contains %zu achievements.", path.c_str(), snap.unlocks.size());

    // On every UNLOCK the .bin copy is always forced (rate-limit stays only
    // for stat saves WITHOUT achievements).
    statscache::BackupStatsBins(job.appId, job.accountId, /*force=*/true);
}

void ProcessBinCopy(const BackupJob& job) {
    g_touched[job.appId].insert(job.accountId);
    statscache::BackupStatsBins(job.appId, job.accountId, job.force);
    if (job.force) {
        AC_LOG_INFO(kModule, "Backup: final/end-of-session stats copy executed for app %u (account %u).", job.appId,
                    job.accountId);
    }
}

void ProcessStatsUpdate(const BackupJob& job) {
    const std::string path = snapshot::SnapshotPath(job.appId, job.accountId);
    if (path.empty())
        return;
    g_touched[job.appId].insert(job.accountId);

    snapshot::SnapshotData snap = snapshot::Load(path);
    const std::uint32_t now = static_cast<std::uint32_t>(std::time(nullptr));
    const std::unordered_set<std::uint32_t>& schemaBuckets = statscache::SchemaBucketsFor(job.appId);

    for (const auto& [id, value] : job.stats) {
        const bool knownStat = snapshot::HasStat(snap, id);

        // Schema-driven reconciliation: when a stat IS an achievement bucket,
        // every set bit must exist in the snapshot. Date: 'now' when the stat
        // stat was already tracked, 0 on first sighting (baseline).
        if (schemaBuckets.count(id) != 0 && value != 0) {
            for (std::uint32_t bit = 0; bit < 32; ++bit) {
                if ((value & (1u << bit)) == 0)
                    continue;
                const std::uint32_t achievementId = id * 32u + bit;
                if (snapshot::HasUnlock(snap, achievementId))
                    continue;
                snap.unlocks.push_back(snapshot::UnlockEntry{achievementId, knownStat ? now : 0});
                AC_LOG_INFO(kModule,
                            "Backup: schema-derived achievement recorded: AppID %u bucket %u bit %u "
                            "(id %u, unlock_time=%s).",
                            job.appId, id, bit, achievementId, knownStat ? "commit time" : "unknown (baseline)");
            }
        }

        snapshot::MergeStat(snap, id, value);
    }
    snapshot::SortAll(snap);
    snapshot::Save(path, job.appId, job.accountId, job.steamId64, snap);
    AC_LOG_DEBUG(kModule, "Backup: snapshot %s now tracks %zu stat(s).", path.c_str(), snap.stats.size());
}

// Copies the .bins of every managed app found in appcache\stats: runs once per
// process, right after Steam starts, to beat the client's login-reconcile
// (which may discard local pending changes).
void ProcessStartupScan() {
    const std::string statsDir = g_state.steamInstallPath + "\\appcache\\stats";
    const std::string pattern = statsDir + "\\UserGameStats_*_*.bin";
    WIN32_FIND_DATAA fd{};
    HANDLE find = FindFirstFileA(pattern.c_str(), &fd);
    if (find == INVALID_HANDLE_VALUE) {
        AC_LOG_DEBUG(kModule, "Backup: startup scan found no UserGameStats files.");
    } else {
        std::size_t copied = 0;
        do {
            // UserGameStats_<account>_<appid>.bin
            const std::string name = fd.cFileName;
            const std::size_t u1 = name.find('_');
            const std::size_t u2 = name.find('_', u1 + 1);
            const std::size_t dot = name.rfind('.');
            if (u1 == std::string::npos || u2 == std::string::npos || dot == std::string::npos) {
                continue;
            }
            // Keep the parsed strings alive while inspecting strtoul's end
            // pointer (a pointer into substr(...).c_str() would dangle).
            const std::string accountText = name.substr(u1 + 1, u2 - u1 - 1);
            const std::string appText = name.substr(u2 + 1, dot - u2 - 1);
            char* end = nullptr;
            const unsigned long account = std::strtoul(accountText.c_str(), &end, 10);
            if (!end || *end != '\0')
                continue;
            const unsigned long app = std::strtoul(appText.c_str(), &end, 10);
            if (!end || *end != '\0' || app == 0)
                continue;
            if (!ac::luadata::HasDepot(static_cast<steam::AppId>(app)))
                continue;

            const auto appId = static_cast<steam::AppId>(app);
            const auto accountId = static_cast<std::uint32_t>(account);
            g_touched[appId].insert(accountId);
            statscache::BackupStatsBins(appId, accountId, /*force=*/true);
            ++copied;
        } while (FindNextFileA(find, &fd));
        FindClose(find);
        AC_LOG_INFO(kModule, "Backup: startup scan copied %zu managed stats file(s).", copied);
    }

    // In the same startup pass: per-account playtime backup.
    playtime::RefreshAllAccounts();
}

// First due job (FIFO order among due ones). In shutdown ALL jobs count as
// due: the queue drains, delayed checkpoints included.
std::optional<BackupJob> PopDueJobLocked(SteadyClock::time_point now) {
    for (auto it = g_jobs.begin(); it != g_jobs.end(); ++it) {
        if (g_stopping || it->runAt <= now) {
            BackupJob job = std::move(*it);
            g_jobs.erase(it);
            return job;
        }
    }
    return std::nullopt;
}

SteadyClock::time_point EarliestRunAtLocked() {
    auto earliest = SteadyClock::time_point::max();
    for (const auto& job : g_jobs)
        earliest = std::min(earliest, job.runAt);
    return earliest;
}

// Periodic safety net: re-copies the .bins of every (app, account) touched in
// session (forced: rate-limit would apply only to saves inside jobs) and
// refreshes playtime. The monotone guard blocks backup regressions.
void PeriodicCheckpoint() {
    std::size_t pairs = 0;
    // g_touched is worker-exclusive ownership: no lock needed.
    for (const auto& [appId, accounts] : g_touched) {
        for (const auto& accountId : accounts) {
            statscache::BackupStatsBins(appId, accountId, /*force=*/true);
            ++pairs;
        }
    }
    playtime::RefreshAllAccounts();
    if (pairs > 0) {
        AC_LOG_INFO(kModule, "Backup: periodic checkpoint completed (%zu (app,account) pair(s), %zu app(s)).", pairs,
                    g_touched.size());
    } else {
        AC_LOG_DEBUG(kModule, "Backup: periodic checkpoint: nothing touched yet this session.");
    }
}

// A bad file/job must not strand the remaining queue or skip final backups.
template <class F> void RunBackupTask(const char* stage, F&& task) {
    try {
        task();
    } catch (const std::exception& error) {
        ++g_failedJobs;
        AC_LOG_WARN(kModule, "Backup: %s failed: %s; continuing drain.", stage, error.what());
    } catch (...) {
        ++g_failedJobs;
        AC_LOG_WARN(kModule, "Backup: %s failed with unknown exception; continuing drain.", stage);
    }
}

void WorkerLoop(std::atomic<bool>& stop) {
    auto nextCheckpoint = SteadyClock::now() + kCheckpointInterval;
    for (;;) {
        BackupJob job;
        bool hasJob = false;
        bool doCheckpoint = false;
        {
            std::unique_lock<std::mutex> lock(g_workerMutex);
            while (!hasJob && !doCheckpoint) {
                // Central shutdown happens before AchievementModule::Shutdown.
                // Seal and drain here too: otherwise the registry join would
                // wait forever for the later FlushOnShutdown call.
                if (stop.load() && !g_stopping) {
                    g_stopping = true;
                    AC_LOG_INFO(kModule, "Backup: registry stop; draining %zu queued job(s).", g_jobs.size());
                }
                if (auto due = PopDueJobLocked(SteadyClock::now())) {
                    job = std::move(*due);
                    hasJob = true;
                    break;
                }
                if (g_stopping)
                    break; // queue drained: exit before the final copies
                auto wakeAt = nextCheckpoint;
                if (!g_jobs.empty())
                    wakeAt = std::min(wakeAt, EarliestRunAtLocked());
                // The registry stop flag has no CV notification. Bound only
                // that observation latency; disk flush remains fully blocking.
                wakeAt = std::min(wakeAt, SteadyClock::now() + std::chrono::milliseconds(250));
                g_workerCv.wait_until(lock, wakeAt);
                if (SteadyClock::now() >= nextCheckpoint) {
                    nextCheckpoint = SteadyClock::now() + kCheckpointInterval;
                    doCheckpoint = true;
                }
            }
        }
        if (hasJob) {
            RunBackupTask("queued job", [&] {
                if (job.type == JobType::Unlock)
                    ProcessUnlock(job);
                else if (job.type == JobType::BinCopy)
                    ProcessBinCopy(job);
                else if (job.type == JobType::StatsUpdate)
                    ProcessStatsUpdate(job);
                else if (job.type == JobType::StartupScan)
                    ProcessStartupScan();
            });
        } else if (doCheckpoint) {
            RunBackupTask("periodic checkpoint", [] { PeriodicCheckpoint(); });
        } else {
            break;
        }
    }

    // Final .bin copies for every (app, account) seen in session: Steam is
    // now closing and its cache includes the latest unlocks too.
    for (const auto& [appId, accounts] : g_touched) {
        for (const auto& accountId : accounts) {
            RunBackupTask("final stats copy", [&] { statscache::BackupStatsBins(appId, accountId, /*force=*/true); });
        }
    }
    // End-of-session playtime too (includes the minutes just played).
    RunBackupTask("final playtime copy", [] { playtime::RefreshAllAccounts(); });
    AC_LOG_INFO(kModule, "Backup: queue drained and final copies attempted (%zu failed task(s)).", g_failedJobs);
}

bool EnsureWorkerLocked() {
    if (g_worker.valid())
        return true;
    if (!workers::StartWorker("achievement_backup", WorkerLoop, &g_worker)) {
        g_stopping = true;
        AC_LOG_WARN(kModule, "Backup: worker registration refused (registry stopped).");
        return false;
    }
    AC_LOG_DEBUG(kModule, "Backup: async worker registered (I/O off the network thread).");
    return true;
}

// Enqueues a job; false = refused because shutdown is already underway
// (the caller decides whether/logs the warning).
bool EnqueueJob(BackupJob job) {
    {
        std::lock_guard<std::mutex> lock(g_workerMutex);
        if (g_stopping)
            return false;
        if (!EnsureWorkerLocked())
            return false;
        g_jobs.push_back(std::move(job));
    }
    g_workerCv.notify_one();
    return true;
}

BackupJob MakeJob(JobType type, steam::AppId appId, std::uint64_t steamId64) {
    BackupJob job;
    job.type = type;
    job.appId = appId;
    job.accountId = static_cast<std::uint32_t>(steamId64 & 0xFFFFFFFFull);
    job.steamId64 = steamId64;
    return job;
}

} // namespace

std::string FormatUnixTime(std::uint64_t unixTime) { return io::FormatUnixTime(unixTime); }

void RecordUnlock(steam::AppId appId, std::uint64_t steamId64, std::uint32_t achievementId, std::uint32_t unlockTime) {
    BackupJob job = MakeJob(JobType::Unlock, appId, steamId64);
    job.achievementId = achievementId;
    job.unlockTime = unlockTime;
    if (!EnqueueJob(std::move(job))) {
        // Shutdown already underway: the unlock was still logged by the wire
        // module; persistence can no longer be guaranteed here.
        AC_LOG_WARN(kModule, "Backup: unlock %u received during shutdown, not saved.", achievementId);
    }
}

void TouchSession(steam::AppId appId, std::uint64_t steamId64) {
    {
        std::lock_guard<std::mutex> lock(g_touchMutex);
        if (!g_sessionTouched.insert(appId).second)
            return; // already touched
    }
    EnqueueJob(MakeJob(JobType::BinCopy, appId, steamId64));
}

void SessionEnded(steam::AppId appId, std::uint64_t steamId64) {
    if (appId == 0 || steamId64 == 0)
        return;
    if (!ac::luadata::HasDepot(appId)) {
        AC_LOG_DEBUG(kModule, "Backup: session end for unmanaged app %u; no final backup needed.", appId);
        return;
    }
    BackupJob job = MakeJob(JobType::BinCopy, appId, steamId64);
    job.force = true;
    job.runAt = SteadyClock::now() + kSessionEndDelay;
    AC_LOG_INFO(kModule,
                "Game session ended for app %u: final stats backup scheduled in %lld s "
                "(catches Steam's post-exit cache write).",
                appId, static_cast<long long>(kSessionEndDelay.count()));
    if (!EnqueueJob(std::move(job))) {
        AC_LOG_WARN(kModule, "Backup: session-end backup for app %u rejected (shutdown in progress).", appId);
    }
}

void BackupAllKnownStatsAtStartup() { EnqueueJob(MakeJob(JobType::StartupScan, 0, 0)); }

void RecordStats(steam::AppId appId, std::uint64_t steamId64,
                 const std::vector<std::pair<std::uint32_t, std::uint32_t>>& stats) {
    BackupJob job = MakeJob(JobType::StatsUpdate, appId, steamId64);
    job.stats = stats;
    EnqueueJob(std::move(job)); // silent refusal in shutdown: best-effort mirror
}

void FlushOnShutdown() {
    {
        std::lock_guard<std::mutex> lock(g_workerMutex);
        g_stopping = true;
    }
    g_workerCv.notify_all();

    workers::WorkerCompletion completion;
    {
        std::lock_guard<std::mutex> lock(g_workerMutex);
        completion = g_worker;
        AC_LOG_INFO(kModule, "Backup: flush requested (%zu queued job(s)).", g_jobs.size());
    }
    // No queue lock while waiting; safe before OR after registry shutdown.
    if (completion.valid())
        completion.wait();
    if (g_failedJobs != 0) {
        AC_LOG_WARN(kModule, "Backup: flush completed with %zu failed task(s); see earlier warnings.", g_failedJobs);
    } else {
        AC_LOG_INFO(kModule, "Backup: flush worker completed.");
    }
}

} // namespace ac::hooks::AchievementBackup
