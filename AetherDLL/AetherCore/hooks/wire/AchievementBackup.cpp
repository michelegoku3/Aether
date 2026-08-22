#include "pch.h"
#include "hooks/wire/AchievementBackup.h"

#include <algorithm>
#include <bit>
#include <condition_variable>
#include <iterator>
#include <cstdio>
#include <ctime>
#include <deque>
#include <fstream>
#include <mutex>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include <cstdlib>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"

namespace ac::hooks::AchievementBackup {
namespace {

constexpr const char* kModule = "Wire.Achievement";

// ---------------------------------------------------------------------------
// Helper di formattazione tempo (ora locale).
// ---------------------------------------------------------------------------

std::tm LocalTime(std::time_t tt) {
    std::tm tmBuf{};
#if defined(_WIN32)
    localtime_s(&tmBuf, &tt);
#else
    localtime_r(&tt, &tmBuf);
#endif
    return tmBuf;
}

std::string FormatWallClockNow() {
    const std::tm tmBuf = LocalTime(std::time(nullptr));
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%04d-%02d-%02dT%02d:%02d:%02d",
                  tmBuf.tm_year + 1900, tmBuf.tm_mon + 1, tmBuf.tm_mday,
                  tmBuf.tm_hour, tmBuf.tm_min, tmBuf.tm_sec);
    return buf;
}

// ---------------------------------------------------------------------------
// Risoluzione percorsi: sezione Backup di AetherDesk se disponibile.
// ---------------------------------------------------------------------------

// Legge <steam>\aethercore\desk_path.cfg (scritto da AetherDesk): prima riga
// = percorso di AetherData. Letto UNA volta per processo (il file non cambia
// mentre Steam è attivo: lo riscrive AetherDesk al proprio avvio): rileggerlo
// a ogni unlock/copia sarebbe I/O inutile.
std::string CachedDeskDataDir() {
    static std::mutex cacheMutex;
    static std::string cached;
    static bool resolved = false;
    std::lock_guard<std::mutex> lock(cacheMutex);
    if (!resolved) {
        std::ifstream ifs(g_state.aetherCoreDir + "\\desk_path.cfg");
        if (ifs.is_open()) {
            std::string line;
            if (std::getline(ifs, line)) {
                while (!line.empty() &&
                       (line.back() == '\r' || line.back() == '\n' || line.back() == ' ')) {
                    line.pop_back();
                }
                cached = line;
            }
        }
        resolved = true;
    }
    return cached;
}

// Cartella backup per l'app: SOLO nella sezione Backup di AetherDesk
// (risolta via desk_path.cfg). Senza AetherDesk non si fa backup: il fallback
// in aethercore è stato rimosso su richiesta (nessun dato utente nel folder
// di Steam\aethercore). Ritorna "" quando il backup non è disponibile.
std::string BackupDirForApp(steam::AppId appId) {
    const std::string deskData = CachedDeskDataDir();
    if (deskData.empty()) {
        AC_LOG_WARN_ONCE(kModule,
                         "Backup: AetherData path unknown (desk_path.cfg missing): "
                         "achievement backup disabled for this session.");
        return {};
    }
    const std::string app = std::to_string(appId);
    const std::string root = deskData + "\\backup";
    CreateDirectoryA(root.c_str(), nullptr);
    CreateDirectoryA((root + "\\" + app).c_str(), nullptr);
    const std::string dir = root + "\\" + app + "\\achievements";
    CreateDirectoryA(dir.c_str(), nullptr);
    return dir;
}

// ---------------------------------------------------------------------------
// Snapshot JSON: formato "aether-achievement-mirror-v1", un oggetto per riga
// dentro "achievements" (vedi SaveSnapshot) così il load è un semplice scan
// di righe senza dipendere da un parser JSON completo.
// ---------------------------------------------------------------------------

struct UnlockEntry {
    std::uint32_t id = 0;          // achievement_id del protocollo (eMsg 5466)
    std::uint32_t unlockTime = 0;  // unix time
};

struct StatEntry {
    std::uint32_t id = 0;     // stat_id del protocollo (per le bitfield = bucket)
    std::uint32_t value = 0;  // ultimo valore committato
};

struct SnapshotData {
    std::vector<UnlockEntry> unlocks;
    std::vector<StatEntry> stats;
};

SnapshotData LoadSnapshot(const std::string& path) {
    SnapshotData out;
    std::ifstream ifs(path);
    if (!ifs.is_open()) return out;
    std::string line;
    while (std::getline(ifs, line)) {
        if (const std::size_t p = line.find("{\"id\":"); p != std::string::npos) {
            UnlockEntry e{};
            if (std::sscanf(line.c_str() + p, "{\"id\": %u, \"unlock_time\": %u", &e.id, &e.unlockTime) == 2) {
                out.unlocks.push_back(e);
            }
            continue;
        }
        if (const std::size_t p = line.find("{\"sid\":"); p != std::string::npos) {
            StatEntry st{};
            if (std::sscanf(line.c_str() + p, "{\"sid\": %u, \"value\": %u", &st.id, &st.value) == 2) {
                out.stats.push_back(st);
            }
        }
    }
    return out;
}

// Riscrittura atomica: prima su .tmp, poi MoveFileEx sopra il file precedente.
void SaveSnapshot(const std::string& path, steam::AppId appId, std::uint32_t accountId,
                  std::uint64_t steamId64, const std::vector<UnlockEntry>& entries,
                  const std::vector<StatEntry>& stats) {
    const std::string tmp = path + ".tmp";
    {
        std::ofstream out(tmp, std::ios::trunc);
        if (!out.is_open()) {
            AC_LOG_WARN(kModule, "Backup: cannot write %s.", tmp.c_str());
            return;
        }
        out << "{\n";
        out << "  \"_format\": \"aether-achievement-mirror-v1\",\n";
        out << "  \"_note\": \"snapshot generated by AetherDLL; rebuild the .bin with Tools/achievement_decoder.py rebuild\",\n";
        out << "  \"appid\": " << appId << ",\n";
        out << "  \"account_id\": " << accountId << ",\n";
        out << "  \"steamid64\": " << steamId64 << ",\n";
        out << "  \"updated\": \"" << FormatWallClockNow() << "\",\n";
        out << "  \"updated_unix\": " << static_cast<long long>(std::time(nullptr)) << ",\n";
        out << "  \"achievements\": [\n";
        for (std::size_t i = 0; i < entries.size(); ++i) {
            // bucket/bit derivati dalla convenzione id = bucket*32 + bit
            // (validata incrociando schema e cache); il tool rebuild le ricontrolla.
            out << "    {\"id\": " << entries[i].id
                << ", \"unlock_time\": " << entries[i].unlockTime
                << ", \"unlocked_at\": \"" << FormatUnixTime(entries[i].unlockTime) << "\""
                << ", \"bucket\": " << (entries[i].id / 32u)
                << ", \"bit\": " << (entries[i].id % 32u) << "}"
                << (i + 1 < entries.size() ? "," : "") << "\n";
        }
        out << "  ],\n";
        out << "  \"stats\": [\n";
        for (std::size_t i = 0; i < stats.size(); ++i) {
            out << "    {\"sid\": " << stats[i].id
                << ", \"value\": " << stats[i].value << "}"
                << (i + 1 < stats.size() ? "," : "") << "\n";
        }
        out << "  ]\n";
        out << "}\n";
        out.flush();
    }
    if (!MoveFileExA(tmp.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING)) {
        AC_LOG_WARN(kModule, "Backup: cannot replace %s (the .tmp file remains on disk).", path.c_str());
    }
}

// ---------------------------------------------------------------------------
// Copia dei .bin della cache di Steam (stats account + schema).
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Valutazione "ricchezza" di un file UserGameStats: conta gli achievement
// come somma dei bit impostati nei bitfield `data` di ogni bucket (formato
// binario VDF: 0x00=dict, 0x01=stringa, 0x02=int32, 0x08=chiusura dict).
// Serve al guard monotono: mai sovrascrivere un backup con una cache più
// povera (es. appena azzerata dal login-reconcile). -1 = illeggibile.
// ---------------------------------------------------------------------------

int CountDictAchievements(const std::vector<std::uint8_t>& buf, std::size_t& pos, int depth) {
    int total = 0;
    while (pos < buf.size()) {
        const std::uint8_t type = buf[pos++];
        if (type == 0x08) return total;
        std::size_t keyEnd = pos;
        while (keyEnd < buf.size() && buf[keyEnd] != 0x00) ++keyEnd;
        if (keyEnd >= buf.size()) return total;
        const std::string key(reinterpret_cast<const char*>(&buf[pos]), keyEnd - pos);
        pos = keyEnd + 1;
        if (type == 0x00) {
            total += CountDictAchievements(buf, pos, depth + 1);
        } else if (type == 0x01) {
            std::size_t valEnd = pos;
            while (valEnd < buf.size() && buf[valEnd] != 0x00) ++valEnd;
            pos = valEnd + 1;
        } else if (type == 0x02) {
            if (pos + 4 > buf.size()) return total;
            if (key == "data") {
                std::uint32_t bits = 0;
                for (int b = 0; b < 4; ++b) bits |= static_cast<std::uint32_t>(buf[pos + b]) << (8 * b);
                total += std::popcount(bits);
            }
            pos += 4;
        } else if (type == 0x03) {
            pos += 4;
        } else {
            return total;   // tipo sconosciuto: fermati (formato inatteso)
        }
    }
    return total;
}

int CountAchievementsInBinFile(const std::string& path) {
    std::ifstream f(path, std::ios::binary);
    if (!f.is_open()) return -1;
    std::vector<std::uint8_t> buf((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
    std::size_t pos = 0;
    return CountDictAchievements(buf, pos, 0);
}

// Copia i .bin della cache di Steam (stats account + schema). `force` salta il
// rate-limit (usato per le copie finali di shutdown); altrimenti il worker
// limita le copie dello stesso (app, account) a una ogni kBinCopyMinInterval
// per non copiare il file a ogni stat salvata dal gioco.
constexpr auto kBinCopyMinInterval = std::chrono::seconds(60);

// Proprietà esclusiva del worker: ultimo istante di copia per (app, account).
std::unordered_map<std::uint64_t, std::chrono::steady_clock::time_point> g_lastBinCopy;

void BackupStatsBins(steam::AppId appId, std::uint32_t accountId, bool force = false) {
    const std::string statsDir = g_state.steamInstallPath + "\\appcache\\stats";
    const std::string dstDir = BackupDirForApp(appId);
    if (dstDir.empty()) return;   // backup non disponibile (niente AetherData)

    if (!force) {
        const std::uint64_t key = (static_cast<std::uint64_t>(appId) << 32) | accountId;
        const auto now = std::chrono::steady_clock::now();
        auto it = g_lastBinCopy.find(key);
        if (it != g_lastBinCopy.end() && now - it->second < kBinCopyMinInterval) {
            return;   // copiata di recente: salta silenziosamente
        }
        g_lastBinCopy[key] = now;
    }

    const std::string userBin = "UserGameStats_" + std::to_string(accountId) + "_" +
                                std::to_string(appId) + ".bin";
    const std::string schemaBin = "UserGameStatsSchema_" + std::to_string(appId) + ".bin";

    if (GetFileAttributesA((statsDir + "\\" + userBin).c_str()) != INVALID_FILE_ATTRIBUTES) {
        // GUARD MONOTONO: mai sovrascrivere un backup con una cache più povera.
        // Se la cache di Steam è stata appena azzerata (login-reconcile), il
        // file sorgente ha MENO achievement del backup: in quel caso si tiene
        // il backup e si logga un WARN ben visibile. Il backup può solo
        // crescere, mai regredire.
        const std::string dstUserBin = dstDir + "\\" + userBin;
        if (GetFileAttributesA(dstUserBin.c_str()) != INVALID_FILE_ATTRIBUTES) {
            const int srcCount = CountAchievementsInBinFile(statsDir + "\\" + userBin);
            const int dstCount = CountAchievementsInBinFile(dstUserBin);
            if (srcCount >= 0 && dstCount >= 0 && srcCount < dstCount) {
                AC_LOG_WARN(kModule,
                            "Backup: refusing to overwrite %s with an emptier cache "
                            "(source has %d achievement(s), backup has %d) — the Steam cache was "
                            "probably wiped again; keeping the richer backup. Restore it manually "
                            "to recover (Tools\\achievement_decoder.py decode to verify).",
                            userBin.c_str(), srcCount, dstCount);
                // Copia comunque lo schema (non rischia nulla) e termina.
                if (GetFileAttributesA((statsDir + "\\" + schemaBin).c_str()) != INVALID_FILE_ATTRIBUTES) {
                    CopyFileA((statsDir + "\\" + schemaBin).c_str(), (dstDir + "\\" + schemaBin).c_str(), FALSE);
                }
                return;
            }
        }
        if (CopyFileA((statsDir + "\\" + userBin).c_str(), dstUserBin.c_str(), FALSE)) {
            AC_LOG_INFO(kModule, "Backup: copied %s to %s.", userBin.c_str(), dstDir.c_str());
        } else {
            AC_LOG_WARN(kModule, "Backup: copy of %s failed.", userBin.c_str());
        }
    } else {
        AC_LOG_DEBUG(kModule, "Backup: %s not present yet in appcache\\stats.", userBin.c_str());
    }
    if (GetFileAttributesA((statsDir + "\\" + schemaBin).c_str()) != INVALID_FILE_ATTRIBUTES) {
        if (!CopyFileA((statsDir + "\\" + schemaBin).c_str(), (dstDir + "\\" + schemaBin).c_str(), FALSE)) {
            AC_LOG_WARN(kModule, "Backup: copy of %s failed.", schemaBin.c_str());
        }
    }
}

// ---------------------------------------------------------------------------
// Worker asincrono: unico proprietario del disco e della mappa dei (app,
// account) toccati. RecordUnlock()/TouchSession() fanno solo enqueue (veloce,
// nessuna I/O nel thread di rete); tutto il lavoro su file avviene qui.
// ---------------------------------------------------------------------------

enum class JobType {
    Unlock,      // aggiorna lo snapshot JSON + copia i .bin (rate-limited)
    BinCopy,     // copia solo i .bin (es. primo 818/151/5466 della sessione)
    StatsUpdate, // aggiorna la sezione stats dello snapshot JSON
    StartupScan, // snapshot iniziale di tutti i .bin (una volta per processo)
};

struct BackupJob {
    JobType type = JobType::Unlock;
    steam::AppId appId = 0;
    std::uint32_t accountId = 0;
    std::uint64_t steamId64 = 0;
    std::uint32_t achievementId = 0;
    std::uint32_t unlockTime = 0;
    std::vector<std::pair<std::uint32_t, std::uint32_t>> stats;
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

void ProcessUnlock(const BackupJob& job) {
    const std::string dir = BackupDirForApp(job.appId);
    if (dir.empty()) return;   // backup non disponibile (niente AetherData)

    const std::string path = dir + "\\UserGameStats_" + std::to_string(job.accountId) + "_" +
                             std::to_string(job.appId) + ".json";

    g_touched[job.appId].insert(job.accountId);

    // Merge: id già presente -> mantieni il tempo più antico; altrimenti aggiungi.
    SnapshotData snap = LoadSnapshot(path);
    bool found = false;
    for (auto& e : snap.unlocks) {
        if (e.id == job.achievementId) {
            e.unlockTime = (e.unlockTime != 0 && e.unlockTime < job.unlockTime) ? e.unlockTime
                                                                                : job.unlockTime;
            found = true;
            break;
        }
    }
    if (!found) snap.unlocks.push_back(UnlockEntry{job.achievementId, job.unlockTime});

    std::sort(snap.unlocks.begin(), snap.unlocks.end(),
              [](const UnlockEntry& a, const UnlockEntry& b) { return a.id < b.id; });
    SaveSnapshot(path, job.appId, job.accountId, job.steamId64, snap.unlocks, snap.stats);
    AC_LOG_DEBUG(kModule, "Backup: snapshot %s now contains %zu achievements.",
                 path.c_str(), snap.unlocks.size());

    // A ogni SBLOCCO la copia del .bin è sempre forzata (il rate-limit 60s
    // resta solo per i salvataggi di stat SENZA achievement): il JSON è già
    // aggiornato sopra, qui garantiamo anche il .bin il prima possibile.
    BackupStatsBins(job.appId, job.accountId, /*force=*/true);
}

void ProcessBinCopy(const BackupJob& job) {
    g_touched[job.appId].insert(job.accountId);
    BackupStatsBins(job.appId, job.accountId);
}

void ProcessStatsUpdate(const BackupJob& job) {
    const std::string dir = BackupDirForApp(job.appId);
    if (dir.empty()) return;
    const std::string path = dir + "\\UserGameStats_" + std::to_string(job.accountId) + "_" +
                             std::to_string(job.appId) + ".json";
    g_touched[job.appId].insert(job.accountId);

    SnapshotData snap = LoadSnapshot(path);
    for (const auto& [id, value] : job.stats) {
        bool found = false;
        for (auto& st : snap.stats) {
            if (st.id == id) {          // ultimo valore committato vince
                st.value = value;
                found = true;
                break;
            }
        }
        if (!found) snap.stats.push_back(StatEntry{id, value});
    }
    std::sort(snap.stats.begin(), snap.stats.end(),
              [](const StatEntry& a, const StatEntry& b) { return a.id < b.id; });
    SaveSnapshot(path, job.appId, job.accountId, job.steamId64, snap.unlocks, snap.stats);
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
        return;
    }
    std::size_t copied = 0;
    do {
        // UserGameStats_<account>_<appid>.bin
        const std::string name = fd.cFileName;
        const std::size_t u1 = name.find('_');
        const std::size_t u2 = name.find('_', u1 + 1);
        const std::size_t dot = name.rfind('.');
        if (u1 == std::string::npos || u2 == std::string::npos || dot == std::string::npos) continue;
        char* end = nullptr;
        const unsigned long account = std::strtoul(name.substr(u1 + 1, u2 - u1 - 1).c_str(), &end, 10);
        if (!end || *end != '\0') continue;
        const unsigned long app = std::strtoul(name.substr(u2 + 1, dot - u2 - 1).c_str(), &end, 10);
        if (!end || *end != '\0' || app == 0) continue;
        if (!ac::luadata::HasDepot(static_cast<steam::AppId>(app))) continue;

        const auto appId = static_cast<steam::AppId>(app);
        const std::uint32_t accountId = static_cast<std::uint32_t>(account);
        g_touched[appId].insert(accountId);
        BackupStatsBins(appId, accountId, /*force=*/true);
        ++copied;
    } while (FindNextFileA(find, &fd));
    FindClose(find);
    AC_LOG_INFO(kModule, "Backup: startup scan copied %zu managed stats file(s).", copied);
}

void WorkerLoop() {
    for (;;) {
        BackupJob job;
        {
            std::unique_lock<std::mutex> lock(g_workerMutex);
            g_workerCv.wait(lock, [] { return g_stopping || !g_jobs.empty(); });
            if (g_jobs.empty()) {
                if (g_stopping) break;   // coda scaricata: esci prima delle copie finali
                continue;
            }
            job = g_jobs.front();
            g_jobs.pop_front();
        }
        if (job.type == JobType::Unlock) ProcessUnlock(job);
        else if (job.type == JobType::BinCopy) ProcessBinCopy(job);
        else if (job.type == JobType::StatsUpdate) ProcessStatsUpdate(job);
        else if (job.type == JobType::StartupScan) ProcessStartupScan();
    }

    // Copie finali dei .bin per ogni (app, account) visto in sessione: ora
    // Steam sta chiudendo e la sua cache include anche gli ultimi sblocchi.
    for (const auto& [appId, accounts] : g_touched) {
        for (const auto& accountId : accounts) {
            BackupStatsBins(appId, accountId, /*force=*/true);
        }
    }
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

}  // namespace

std::string FormatUnixTime(std::uint64_t unixTime) {
    const std::tm tmBuf = LocalTime(static_cast<std::time_t>(unixTime));
    char buf[40];
    std::snprintf(buf, sizeof(buf), "%02d/%02d/%04d %02d:%02d:%02d",
                  tmBuf.tm_mday, tmBuf.tm_mon + 1, tmBuf.tm_year + 1900,
                  tmBuf.tm_hour, tmBuf.tm_min, tmBuf.tm_sec);
    return buf;
}

void RecordUnlock(steam::AppId appId, std::uint64_t steamId64,
                  std::uint32_t achievementId, std::uint32_t unlockTime) {
    BackupJob job;
    job.type = JobType::Unlock;
    job.appId = appId;
    job.accountId = static_cast<std::uint32_t>(steamId64 & 0xFFFFFFFFull);
    job.steamId64 = steamId64;
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
        if (!g_sessionTouched.insert(appId).second) return;   // già toccato in questa sessione
    }
    BackupJob job;
    job.type = JobType::BinCopy;
    job.appId = appId;
    job.accountId = static_cast<std::uint32_t>(steamId64 & 0xFFFFFFFFull);
    job.steamId64 = steamId64;
    EnqueueJob(std::move(job));
}

void BackupAllKnownStatsAtStartup() {
    BackupJob job;
    job.type = JobType::StartupScan;
    EnqueueJob(std::move(job));
}

void RecordStats(steam::AppId appId, std::uint64_t steamId64,
                 const std::vector<std::pair<std::uint32_t, std::uint32_t>>& stats) {
    BackupJob job;
    job.type = JobType::StatsUpdate;
    job.appId = appId;
    job.accountId = static_cast<std::uint32_t>(steamId64 & 0xFFFFFFFFull);
    job.steamId64 = steamId64;
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
