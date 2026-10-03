#pragma once

#include <algorithm>
#include <cstdint>
#include <string>
#include <vector>

#include "core/SteamTypes.h"

// ============================================================================
// UserStatsSnapshot — store JSON dello stato achievement/stat per
// (account, app): formato "aether-achievement-mirror-v1", un oggetto per riga
// (load = scan di righe, senza parser JSON completo). Possiede le REGOLE DI
// MERGE monotone: il contenuto può solo crescere (0 = data sconosciuta che un
// tempo reale sostituisce; vince il tempo più antico; stat = ultimo valore).
// ============================================================================
namespace ac::backup::snapshot {

struct UnlockEntry {
    std::uint32_t id = 0;          // achievement_id del protocollo (eMsg 5466)
    std::uint32_t unlockTime = 0;  // unix time; 0 = data sconosciuta (baseline)
};

struct StatEntry {
    std::uint32_t id = 0;     // stat_id del protocollo (per le bitfield = bucket)
    std::uint32_t value = 0;  // ultimo valore committato
};

struct SnapshotData {
    std::vector<UnlockEntry> unlocks;
    std::vector<StatEntry> stats;
};

// Percorso del file snapshot per (app, account). "" se il backup non disponibile.
std::string SnapshotPath(steam::AppId appId, std::uint32_t accountId);

// Carica uno snapshot (file assente = vuoto, nessun errore).
SnapshotData Load(const std::string& path);

// Scrittura atomica (.tmp + move) dell'intero snapshot.
void Save(const std::string& path, steam::AppId appId, std::uint32_t accountId,
          std::uint64_t steamId64, const SnapshotData& snap);

// --- Regole di merge monotone (header-only: pure e testabili senza link) ----
// Aggiunge uno sblocco o aggiorna la data (0 viene sostituito da un tempo
// reale; tra due tempi reali vince il più antico).
inline void MergeUnlock(SnapshotData& snap, std::uint32_t achievementId, std::uint32_t unlockTime) {
    for (auto& e : snap.unlocks) {
        if (e.id != achievementId) continue;
        // Regola: vince il tempo PIÙ ANTICO, ma 0 (baseline sconosciuta) viene
        // sostituito da qualsiasi tempo reale.
        if (e.unlockTime == 0) e.unlockTime = unlockTime;
        else if (unlockTime != 0 && unlockTime < e.unlockTime) e.unlockTime = unlockTime;
        return;
    }
    snap.unlocks.push_back(UnlockEntry{achievementId, unlockTime});
}

// Aggiorna il valore di una stat (ultimo valore committato vince).
inline void MergeStat(SnapshotData& snap, std::uint32_t statId, std::uint32_t value) {
    for (auto& st : snap.stats) {
        if (st.id == statId) {
            st.value = value;   // ultimo valore committato vince
            return;
        }
    }
    snap.stats.push_back(StatEntry{statId, value});
}

inline bool HasUnlock(const SnapshotData& snap, std::uint32_t achievementId) {
    return std::any_of(snap.unlocks.begin(), snap.unlocks.end(),
                       [achievementId](const UnlockEntry& e) { return e.id == achievementId; });
}

inline bool HasStat(const SnapshotData& snap, std::uint32_t statId) {
    return std::any_of(snap.stats.begin(), snap.stats.end(),
                       [statId](const StatEntry& st) { return st.id == statId; });
}

// id crescente (output deterministico)
inline void SortAll(SnapshotData& snap) {
    std::sort(snap.unlocks.begin(), snap.unlocks.end(),
              [](const UnlockEntry& a, const UnlockEntry& b) { return a.id < b.id; });
    std::sort(snap.stats.begin(), snap.stats.end(),
              [](const StatEntry& a, const StatEntry& b) { return a.id < b.id; });
}

}  // namespace ac::backup::snapshot
