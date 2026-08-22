#pragma once

#include <cstdint>
#include <string>
#include <utility>
#include <vector>

#include "core/SteamTypes.h"

// ============================================================================
// AchievementBackup — persistenza anti-perdita degli sblocchi achievement.
//
// Perché esiste: il server Steam non conferma mai gli StoreUserStats2 di un
// gioco che l'account non possiede, quindi l'unica copia dei progressi è la
// cache locale del client (appcache\stats\UserGameStats_<account>_<appid>.bin,
// riconoscibile da PendingChanges > 0). Qualunque invalidazione client-side di
// quella cache è una perdita definitiva. Questo modulo ne mantiene una copia
// indipendente, con lo stesso naming per-(account, app) di Steam:
//
//   <AetherData>\backup\<appid>\achievements\            [via desk_path.cfg;
//                                               unica destinazione: senza
//                                               AetherDesk niente backup]
//       UserGameStats_<account>_<appid>.json   snapshot sblocchi (leggibile;
//                                               rigenera il .bin con
//                                               Tools\achievement_decoder.py)
//       UserGameStats_<account>_<appid>.bin    copia della cache Steam
//       UserGameStatsSchema_<appid>.bin        copia dello schema del gioco
//
// Contratto (facciata minima, accoppiamento minimo con AchievementModule):
//   * RecordUnlock() è ASINCRONA: nessuna I/O su disco nel thread di rete di
//     Steam — il lavoro viene accodato a un worker dedicato (avviato al primo
//     sblocco). Lo snapshot viene riscritto in modo atomico (.tmp + move) con
//     merge (id duplicato -> vince il tempo più antico).
//   * FlushOnShutdown() scarica la coda, esegue l'ultima copia dei .bin (a
//     quel punto Steam ha flushato la cache, quindi include anche gli ultimi
//     sblocchi della sessione) e spegne il worker. Da chiamare in Shutdown().
//   * Tutto è best-effort: un errore di I/O viene solo loggato (WARN) e non
//     interferisce mai con il traffico di rete.
// ============================================================================

namespace ac::hooks::AchievementBackup {

// "1787052569" -> "18/08/2026 13:29:29" (ora locale). Condivisa con
// AchievementModule per le righe di log; qui perché descrive sblocchi.
std::string FormatUnixTime(std::uint64_t unixTime);

// Accoda il backup di uno sblocco (vedi contratto sopra). Thread-safe.
void RecordUnlock(steam::AppId appId, std::uint64_t steamId64,
                  std::uint32_t achievementId, std::uint32_t unlockTime);

// Backup "di sessione": al primo utilizzo stats di un app (818, 151 o 5466)
// copia i .bin esistenti della cache Steam, così i progressi accumulati
// vengono protetti anche se la sessione non produce nuovi sblocchi.
// Una sola volta per app per processo di Steam. Thread-safe.
void TouchSession(steam::AppId appId, std::uint64_t steamId64);

// Snapshot iniziale (una volta per processo): copia i .bin di TUTTI gli app
// gestiti presenti in appcache\stats, subito dopo l'avvio di Steam e prima
// che il login-reconcile del client possa scartare i cambi pendenti (è la
// finestra in cui si è verificata la perdita del 21/08). Async.
void BackupAllKnownStatsAtStartup();

// Registra le stat numeriche di un commit (incluse le bitfield achievement:
// molti giochi — es. Spider-Man — committano gli achievement come
// stat_id=<bucket>, value=<bitfield>) nello snapshot JSON. Async.
void RecordStats(steam::AppId appId, std::uint64_t steamId64,
                 const std::vector<std::pair<std::uint32_t, std::uint32_t>>& stats);

// Scarica la coda, copia i .bin un'ultima volta e spegne il worker. Blocca
// finché tutto il lavoro pendente è completato (chiamare solo in shutdown).
void FlushOnShutdown();

}  // namespace ac::hooks::AchievementBackup
