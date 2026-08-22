#pragma once

#include <cstdint>
#include <string>
#include <unordered_set>

#include "core/SteamTypes.h"

// ============================================================================
// SteamStatsCache — accesso ai file cache stats di Steam (appcache\stats):
// copia dei .bin con GUARD MONOTONO (mai sovrascrivere un backup più ricco
// con una cache appena azzerata) e rate-limit; conteggio achievement dei .bin
// binari; interpretazione degli schemi (bucket achievement) per tradurre le
// stat bitfield in achievement.
//
// Stato interno (cache schema per app, rate-limit per (app,account)):
// proprietà del thread worker del facade — non serve concorrenza interna.
// ============================================================================
namespace ac::backup::statscache {

// Copia UserGameStats_<account>_<appid>.bin + UserGameStatsSchema_<appid>.bin
// nel backup. `force` salta il rate-limit (sblocchi e copie finali).
void BackupStatsBins(steam::AppId appId, std::uint32_t accountId, bool force = false);

// Achievement presenti in un .bin UserGameStats (somma dei bit dei bitfield
// "data"); -1 se illeggibile. Usato dal guard monotono.
int CountAchievementsInBinFile(const std::string& path);

// Insieme dei bucket achievement del gioco letti dallo schema
// (UserGameStatsSchema_<appid>.bin; cache per processo). Bucket con "bits"
// vuoto NON sono bucket achievement.
const std::unordered_set<std::uint32_t>& SchemaBucketsFor(steam::AppId appId);

}  // namespace ac::backup::statscache
