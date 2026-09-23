#include "pch.h"
#include "hooks/wire/AchievementBackup.h"

#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <ctime>
#include <deque>
#include <mutex>
#include <optional>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/wire/BackupIo.h"
#include "hooks/wire/PlaytimeMirror.h"
#include "hooks/wire/SteamStatsCache.h"
#include "hooks/wire/UserStatsSnapshot.h"
#include "scripting/LuaData.h"

// ============================================================================
// AchievementBackup — facade anti-perdita (achievement + stats + playtime).
//
// Questo modulo è SOLO il coordinatore asincrono: coda di job + worker thread.
// La persistenza vera vive nei moduli coesi sotto hooks/wire/:
//   * io::         percorsi AetherData, orari, scrittura atomica
//   * snapshot::   store JSON achievement/stat (regole di merge monotone)
//   * statscache:: copia .bin con guard monotono + interpretazione schemi
//   * playtime::   mirror del tempo di gioco da localconfig.vdf
// Contratto pubblico invariato (vedi AchievementBackup.h): RecordUnlock /
// RecordStats / TouchSession / BackupAllKnownStatsAtStartup /
// Le chiamate di accodamento sono non-bloccanti (nessuna I/O nel thread di rete).
// FlushOnShutdown invece drena e fa join: solo fuori dal loader lock.
// ============================================================================

namespace ac::hooks::AchievementBackup {

// Alias corti dei moduli di persistenza (vedi hooks/wire/).
namespace io = ac::backup::io;
namespace snapshot = ac::backup::snapshot;
namespace statscache = ac::backup::statscache;
namespace playtime = ac::backup::playtime;

namespace {

constexpr const char* kModule = "Wire.Achievement";

// Checkpoint periodico (ogni 5 minuti): ricopia forzata dei .bin per ogni
// (app, account) toccato in sessione + refresh del playtime. È la rete di
// sicurezza che limita la perdita massima a 5 minuti anche quando Steam esce
// senza uno shutdown esplicito (il guard monotono protegge da regressioni).
constexpr auto kCheckpointInterval = std::chrono::minutes(5);
// La copia finale a fine sessione parte ritardata: Steam scrive la cache
// stats qualche istante DOPO il frame games_played vuoto.
constexpr auto kSessionEndDelay = std::chrono::seconds(15);

enum class JobType {
    Unlock,      // aggiorna lo snapshot JSON + copia i .bin (forzata)
    BinCopy,     // copia solo i .bin (es. primo 818/151/5466 della sessione)
    StatsUpdate, // aggiorna la sezione stats dello snapshot JSON
    StartupScan, // snapshot iniziale di tutti i .bin + playtime (1 volta/processo)
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
    bool force = false;                      // salta il rate-limit delle copie
    SteadyClock::time_point runAt{};         // epoch = eseguibile subito
};

std::mutex g_workerMutex;
std::condition_variable g_workerCv;
std::deque<BackupJob> g_jobs;
bool g_stopping = false;
std::thread g_worker;   // avviato al primo job, fermato da FlushOnShutdown()

// Proprietà esclusiva del worker (nessun lock necessario).
std::unordered_map<steam::AppId, std::unordered_set<std::uint32_t>> g_touched;

// Dedup per processo di TouchSession: un solo BinCopy per app a sessione di
// Steam (il file non cambia finché il gioco non salva).
std::mutex g_touchMutex;
std::unordered_set<steam::AppId> g_sessionTouched;

// ---------------------------------------------------------------------------
// Esecuzione dei job (solo thread worker: possiede g_touched e il disco).
// ---------------------------------------------------------------------------

void ProcessUnlock(const BackupJob& job) {
    const std::string path = snapshot::SnapshotPath(job.appId, job.accountId);
    if (path.empty()) return;   // backup non disponibile (niente AetherData)

    g_touched[job.appId].insert(job.accountId);

    snapshot::SnapshotData snap = snapshot::Load(path);
    snapshot::MergeUnlock(snap, job.achievementId, job.unlockTime);
    snapshot::SortAll(snap);
    snapshot::Save(path, job.appId, job.accountId, job.steamId64, snap);
    AC_LOG_DEBUG(kModule, "Backup: snapshot %s now contains %zu achievements.",
                 path.c_str(), snap.unlocks.size());

    // A ogni SBLOCCO la copia del .bin è sempre forzata (il rate-limit resta
    // solo per i salvataggi di stat SENZA achievement).
    statscache::BackupStatsBins(job.appId, job.accountId, /*force=*/true);
}

void ProcessBinCopy(const BackupJob& job) {
    g_touched[job.appId].insert(job.accountId);
    statscache::BackupStatsBins(job.appId, job.accountId, job.force);
    if (job.force) {
        AC_LOG_INFO(kModule, "Backup: final/end-of-session stats copy executed for app %u (account %u).",
                    job.appId, job.accountId);
    }
}

void ProcessStatsUpdate(const BackupJob& job) {
    const std::string path = snapshot::SnapshotPath(job.appId, job.accountId);
    if (path.empty()) return;
    g_touched[job.appId].insert(job.accountId);

    snapshot::SnapshotData snap = snapshot::Load(path);
    const std::uint32_t now = static_cast<std::uint32_t>(std::time(nullptr));
    const std::unordered_set<std::uint32_t>& schemaBuckets =
        statscache::SchemaBucketsFor(job.appId);

    for (const auto& [id, value] : job.stats) {
        const bool knownStat = snapshot::HasStat(snap, id);

        // Riconciliazione schema-driven: se la stat È un bucket achievement,
        // ogni bit impostato deve esistere nello snapshot. Data: 'now' se la
        // stat era già tracciata, 0 se primo avvistamento (baseline).
        if (schemaBuckets.count(id) != 0 && value != 0) {
            for (std::uint32_t bit = 0; bit < 32; ++bit) {
                if ((value & (1u << bit)) == 0) continue;
                const std::uint32_t achievementId = id * 32u + bit;
                if (snapshot::HasUnlock(snap, achievementId)) continue;
                snap.unlocks.push_back(
                    snapshot::UnlockEntry{achievementId, knownStat ? now : 0});
                AC_LOG_INFO(kModule,
                            "Backup: schema-derived achievement recorded: AppID %u bucket %u bit %u "
                            "(id %u, unlock_time=%s).",
                            job.appId, id, bit, achievementId,
                            knownStat ? "commit time" : "unknown (baseline)");
            }
        }

        snapshot::MergeStat(snap, id, value);
    }
    snapshot::SortAll(snap);
    snapshot::Save(path, job.appId, job.accountId, job.steamId64, snap);
    AC_LOG_DEBUG(kModule, "Backup: snapshot %s now tracks %zu stat(s).",
                 path.c_str(), snap.stats.size());
}

// Copia i .bin di tutti gli app gestiti trovati in appcache\stats: gira una
// volta per processo, subito dopo l'avvio di Steam, per battere sul tempo il
// login-reconcile del client (che può scartare i cambi pendenti locali).
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
            if (u1 == std::string::npos || u2 == std::string::npos ||
                dot == std::string::npos) {
                continue;
            }
            char* end = nullptr;
            const unsigned long account =
                std::strtoul(name.substr(u1 + 1, u2 - u1 - 1).c_str(), &end, 10);
            if (!end || *end != '\0') continue;
            const unsigned long app =
                std::strtoul(name.substr(u2 + 1, dot - u2 - 1).c_str(), &end, 10);
            if (!end || *end != '\0' || app == 0) continue;
            if (!ac::luadata::HasDepot(static_cast<steam::AppId>(app))) continue;

            const auto appId = static_cast<steam::AppId>(app);
            const auto accountId = static_cast<std::uint32_t>(account);
            g_touched[appId].insert(accountId);
            statscache::BackupStatsBins(appId, accountId, /*force=*/true);
            ++copied;
        } while (FindNextFileA(find, &fd));
        FindClose(find);
        AC_LOG_INFO(kModule, "Backup: startup scan copied %zu managed stats file(s).", copied);
    }

    // Nella stessa passata di avvio: backup del playtime per account.
    playtime::RefreshAllAccounts();
}

// Primo job scaduto (ordine FIFO tra i dovuti). In shutdown TUTTI i job sono
// considerati dovuti: la coda si drena, inclusi i checkpoint ritardati.
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
    for (const auto& job : g_jobs) earliest = std::min(earliest, job.runAt);
    return earliest;
}

// Rete di sicurezza periodica: ricopia i .bin di ogni (app, account) toccato
// in sessione (forzata: il rate-limit varrebbe solo per i salvataggi nei job)
// e aggiorna il playtime. Il guard monotono impedisce regressioni del backup.
void PeriodicCheckpoint() {
    std::size_t pairs = 0;
    // g_touched è proprietà esclusiva del worker: nessun lock necessario.
    for (const auto& [appId, accounts] : g_touched) {
        for (const auto& accountId : accounts) {
            statscache::BackupStatsBins(appId, accountId, /*force=*/true);
            ++pairs;
        }
    }
    playtime::RefreshAllAccounts();
    if (pairs > 0) {
        AC_LOG_INFO(kModule, "Backup: periodic checkpoint completed (%zu (app,account) pair(s), %zu app(s)).",
                    pairs, g_touched.size());
    } else {
        AC_LOG_DEBUG(kModule, "Backup: periodic checkpoint: nothing touched yet this session.");
    }
}

void WorkerLoop() {
    auto nextCheckpoint = SteadyClock::now() + kCheckpointInterval;
    for (;;) {
        BackupJob job;
        bool hasJob = false;
        bool doCheckpoint = false;
        {
            std::unique_lock<std::mutex> lock(g_workerMutex);
            while (!hasJob && !doCheckpoint) {
                if (auto due = PopDueJobLocked(SteadyClock::now())) {
                    job = std::move(*due);
                    hasJob = true;
                    break;
                }
                if (g_stopping) break;   // coda scaricata: esci prima delle copie finali
                auto wakeAt = nextCheckpoint;
                if (!g_jobs.empty()) wakeAt = std::min(wakeAt, EarliestRunAtLocked());
                g_workerCv.wait_until(lock, wakeAt);
                if (SteadyClock::now() >= nextCheckpoint) {
                    nextCheckpoint = SteadyClock::now() + kCheckpointInterval;
                    doCheckpoint = true;
                }
            }
        }
        if (hasJob) {
            if (job.type == JobType::Unlock) ProcessUnlock(job);
            else if (job.type == JobType::BinCopy) ProcessBinCopy(job);
            else if (job.type == JobType::StatsUpdate) ProcessStatsUpdate(job);
            else if (job.type == JobType::StartupScan) ProcessStartupScan();
        } else if (doCheckpoint) {
            PeriodicCheckpoint();
        } else {
            break;
        }
    }

    // Copie finali dei .bin per ogni (app, account) visto in sessione: ora
    // Steam sta chiudendo e la sua cache include anche gli ultimi sblocchi.
    for (const auto& [appId, accounts] : g_touched) {
        for (const auto& accountId : accounts) {
            statscache::BackupStatsBins(appId, accountId, /*force=*/true);
        }
    }
    // Anche il playtime di fine sessione (include i minuti appena giocati).
    playtime::RefreshAllAccounts();
}

void EnsureWorkerLocked() {
    if (!g_worker.joinable()) {
        g_worker = std::thread(WorkerLoop);
        AC_LOG_DEBUG(kModule, "Backup: async worker started (I/O off the network thread).");
    }
}

// Accoda un job; false = rifiutato perché lo shutdown è già in corso
// (il chiamante decide se/logga l'avviso).
bool EnqueueJob(BackupJob job) {
    {
        std::lock_guard<std::mutex> lock(g_workerMutex);
        if (g_stopping) return false;
        EnsureWorkerLocked();
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

}  // namespace

std::string FormatUnixTime(std::uint64_t unixTime) {
    return io::FormatUnixTime(unixTime);
}

void RecordUnlock(steam::AppId appId, std::uint64_t steamId64,
                  std::uint32_t achievementId, std::uint32_t unlockTime) {
    BackupJob job = MakeJob(JobType::Unlock, appId, steamId64);
    job.achievementId = achievementId;
    job.unlockTime = unlockTime;
    if (!EnqueueJob(std::move(job))) {
        // Shutdown già in corso: l'unlock è stato loggato comunque dal
        // modulo wire; qui non possiamo più garantire la persistenza.
        AC_LOG_WARN(kModule, "Backup: unlock %u received during shutdown, not saved.",
                    achievementId);
    }
}

void TouchSession(steam::AppId appId, std::uint64_t steamId64) {
    {
        std::lock_guard<std::mutex> lock(g_touchMutex);
        if (!g_sessionTouched.insert(appId).second) return;   // già toccato
    }
    EnqueueJob(MakeJob(JobType::BinCopy, appId, steamId64));
}

void SessionEnded(steam::AppId appId, std::uint64_t steamId64) {
    if (appId == 0 || steamId64 == 0) return;
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

void BackupAllKnownStatsAtStartup() {
    EnqueueJob(MakeJob(JobType::StartupScan, 0, 0));
}

void RecordStats(steam::AppId appId, std::uint64_t steamId64,
                 const std::vector<std::pair<std::uint32_t, std::uint32_t>>& stats) {
    BackupJob job = MakeJob(JobType::StatsUpdate, appId, steamId64);
    job.stats = stats;
    EnqueueJob(std::move(job));   // rifiuto silenzioso in shutdown: mirror best-effort
}

void FlushOnShutdown() {
    {
        std::lock_guard<std::mutex> lock(g_workerMutex);
        g_stopping = true;
    }
    g_workerCv.notify_all();

    // Sposta il thread fuori dalla struttura globale PRIMA del join: così il
    // join avviene senza tenere il mutex (evitare lock+join previene deadlock
    // se il worker deve ancora acquisirlo per l'ultima volta).
    std::thread worker;
    {
        std::lock_guard<std::mutex> lock(g_workerMutex);
        if (g_worker.joinable()) worker = std::move(g_worker);
    }
    if (worker.joinable()) worker.join();   // il worker scarica la coda,
                                            // esegue le copie finali ed esce
}

}  // namespace ac::hooks::AchievementBackup
