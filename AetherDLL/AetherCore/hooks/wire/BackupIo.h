#pragma once

#include <cstdint>
#include <string>

#include "core/SteamTypes.h"

// ============================================================================
// BackupIo — utility condivise del sottosistema backup (AetherData).
//
// Piccolo modulo infrastrutturale usato da tutti gli store del backup:
// formattazione orari locali, risoluzione (con cache per processo) dei
// percorsi AetherData e scrittura atomica dei file generati.
// Nessuna dipendenza dal modulo wire: basso accoppiamento, alta riutilizzo.
// ============================================================================
namespace ac::backup::io {

// "1787052569" -> "18/08/2026 13:29:29" (ora locale).
std::string FormatUnixTime(std::uint64_t unixTime);

// "2026-08-22T10:57:16" (ora locale): intestazione dei file generati.
std::string FormatWallClockNow();

// Percorso di AetherData da <steam>\aethercore\desk_path.cfg, letto UNA volta
// per processo (il file non cambia mentre Steam è attivo). "" se assente.
std::string CachedDeskDataDir();

// <AetherData>\backup\<appid>\achievements (crea l'albero). "" se il backup
// non è disponibile (niente AetherDesk).
std::string BackupDirForApp(steam::AppId appId);

// <AetherData>\backup\playtime (crea la cartella). "" se non disponibile.
std::string BackupPlaytimeDir();

// Sostituzione atomica: tmp -> dst con MOVEFILE_REPLACE_EXISTING.
bool AtomicReplace(const std::string& tmp, const std::string& dst);

}  // namespace ac::backup::io
