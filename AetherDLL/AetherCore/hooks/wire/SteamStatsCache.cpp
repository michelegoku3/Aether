#include "pch.h"
#include "hooks/wire/SteamStatsCache.h"

#include <bit>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <functional>
#include <unordered_map>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/KeyValues.h"
#include "hooks/wire/BackupIo.h"

namespace ac::backup::statscache {
namespace {

constexpr const char* kModule = "Wire.Achievement";

    // ---------------------------------------------------------------------------
    // Copia dei .bin della cache di Steam (stats account + schema).
    // ---------------------------------------------------------------------------

    // ---------------------------------------------------------------------------
    // Interpretazione SCHEMA-DRIVEN delle stat bitfield.
    //
    // Il metodo sicuro per sapere se una stat è un bucket achievement è leggere lo
    // schema del gioco (appcache\stats\UserGameStatsSchema_<appid>.bin): la
    // sezione "stats" contiene un dizionario per ogni bucket, e i bucket con un
    // sotto-dizionario "bits" sono bitfield di achievement. Questo elimina ogni
    // euristica: Stanley usa il bucket 3 con bit 0-4 (ignorati dalla soglia
    // "bit >= 8" del log veloce), Spider-Man i bucket 0/200/201.
    //
    // Cache per app: insieme dei bucket achievement (possibilmente vuoto se lo
    // schema non esiste). Proprietà esclusiva del worker.
    // ---------------------------------------------------------------------------
    std::unordered_map<steam::AppId, std::unordered_set<std::uint32_t>> g_schemaBuckets;
    std::unordered_set<steam::AppId> g_schemaParsed;

    // Estrae i bucket achievement (chiavi della sezione "stats" che contengono
    // una sotto-sezione "bits"). Formato schema: <appid> { stats { <bucket> {
    // bits { ... } } } }: si raccogliono le chiavi dei dizionari a profondità 2
    // che contengono "bits".
    // Bucket achievement via walker KV1 condiviso (utils/KeyValues, P11):
    // sezione "stats" a profondità 1; un bucket (profondità 2) è achievement
    // solo se il suo dizionario "bits" ha almeno una voce.
    struct SchemaBucketWalker final : kv1::Visitor {
        std::unordered_set<std::uint32_t> buckets;
        bool inStats = false;
        int bucketDepth = -1;          // profondità dell'entry del bucket corrente
        std::string currentBucket;
        int bitsEntries = 0;

        void FinalizeBucket() {
            if (inStats && bitsEntries > 0) {
                char* end = nullptr;
                unsigned long id = std::strtoul(currentBucket.c_str(), &end, 10);
                if (end && *end == '\0') buckets.insert(static_cast<std::uint32_t>(id));
            }
            bucketDepth = -1;
            bitsEntries = 0;
        }
        void OnDictBegin(const std::string& name, int depth) override {
            if (depth == 1 && name == "stats") inStats = true;
            if (inStats && depth == 2) {
                currentBucket = name;   // potenziale bucket
                bucketDepth = 2;
                bitsEntries = 0;
            }
            // Semantica identica al parser storico: contano solo le voci
            // DIZIONARIO dentro "bits" (profondità 4).
            if (bucketDepth == 2 && depth == 4) ++bitsEntries;
        }
        void OnDictEnd(int depth) override {
            if (bucketDepth == 2 && depth == 2) FinalizeBucket();
            if (inStats && depth == 1) inStats = false;   // chiusa la sezione "stats"
        }
    };

    std::unordered_set<std::uint32_t> ParseSchemaBuckets(const std::string& path) {
        std::ifstream f(path, std::ios::binary);
        if (!f.is_open()) return {};
        std::vector<std::uint8_t> buf((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
        SchemaBucketWalker walker;
        kv1::WalkBinary(buf.data(), buf.size(), walker);
        return std::move(walker.buckets);
    }

    const std::unordered_set<std::uint32_t>& SchemaBucketsForImpl(steam::AppId appId) {
        if (g_schemaParsed.insert(appId).second) {
            const std::string path = g_state.steamInstallPath + "\\appcache\\stats\\UserGameStatsSchema_" +
                std::to_string(appId) + ".bin";
            g_schemaBuckets[appId] = ParseSchemaBuckets(path);
            AC_LOG_DEBUG(kModule, "Backup: schema for AppID %u -> %zu achievement bucket(s).",
                appId, g_schemaBuckets[appId].size());
        }
        return g_schemaBuckets[appId];
    }


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
            }
            else if (type == 0x01) {
                std::size_t valEnd = pos;
                while (valEnd < buf.size() && buf[valEnd] != 0x00) ++valEnd;
                pos = valEnd + 1;
            }
            else if (type == 0x02) {
                if (pos + 4 > buf.size()) return total;
                if (key == "data") {
                    std::uint32_t bits = 0;
                    for (int b = 0; b < 4; ++b) bits |= static_cast<std::uint32_t>(buf[pos + b]) << (8 * b);
                    total += std::popcount(bits);
                }
                pos += 4;
            }
            else if (type == 0x03) {
                pos += 4;
            }
            else {
                return total;   // tipo sconosciuto: fermati (formato inatteso)
            }
        }
        return total;
    }

    int CountAchievementsInBinFileImpl(const std::string& path) {
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

    void BackupStatsBinsImpl(steam::AppId appId, std::uint32_t accountId, bool force = false) {
        const std::string statsDir = g_state.steamInstallPath + "\\appcache\\stats";
        const std::string dstDir = io::BackupDirForApp(appId);
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
            }
            else {
                AC_LOG_WARN(kModule, "Backup: copy of %s failed.", userBin.c_str());
            }
        }
        else {
            AC_LOG_DEBUG(kModule, "Backup: %s not present yet in appcache\\stats.", userBin.c_str());
        }
        if (GetFileAttributesA((statsDir + "\\" + schemaBin).c_str()) != INVALID_FILE_ATTRIBUTES) {
            if (!CopyFileA((statsDir + "\\" + schemaBin).c_str(), (dstDir + "\\" + schemaBin).c_str(), FALSE)) {
                AC_LOG_WARN(kModule, "Backup: copy of %s failed.", schemaBin.c_str());
            }
        }
    }


}  // namespace

void BackupStatsBins(steam::AppId appId, std::uint32_t accountId, bool force) {
    BackupStatsBinsImpl(appId, accountId, force);
}

int CountAchievementsInBinFile(const std::string& path) {
    return CountAchievementsInBinFileImpl(path);
}

const std::unordered_set<std::uint32_t>& SchemaBucketsFor(steam::AppId appId) {
    return SchemaBucketsForImpl(appId);
}

}  // namespace ac::backup::statscache
