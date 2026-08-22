#pragma once

// ============================================================================
// PlaytimeMirror — backup del tempo di gioco per account.
//
// Fonte: userdata\<account>\config\localconfig.vdf (sezione
// Software/Valve/Steam/"Apps", valori in MINUTI: Playtime, Playtime2wks,
// PlaytimeDisconnected, LastPlayed + autocloud {lastlaunch,lastexit}).
// Dati client-side per i giochi gestiti: stessa fragilità degli achievement.
//
// Output: <AetherData>\backup\playtime\UserPlaytime_<account>.json con
// merge MONOTONO (per ogni campo vince il massimo storico).
// ============================================================================
namespace ac::backup::playtime {

// Rilegge il localconfig di OGNI cartella userdata\* trovata e aggiorna i
// rispettivi snapshot. Best-effort: errori solo loggati.
void RefreshAllAccounts();

}  // namespace ac::backup::playtime
