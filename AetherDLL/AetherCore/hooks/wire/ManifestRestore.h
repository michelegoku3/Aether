#pragma once

// ============================================================================
// ManifestRestore — refill di Steam\depotcache dai backup all'avvio di Steam.
//
// Perché esiste: disinstallare un gioco cancella i suoi .manifest da
// Steam\depotcache, e Steam non serve più i manifest senza autenticazione —
// senza una copia locale il gioco non è più riscaricabile. AetherDesk salva
// ogni .manifest scaricato (store o import locale) in:
//
//   <AetherData>\backup\<app_id>\lua\*.manifest
//
// Questo modulo, una volta per processo di Steam, ricopia in depotcache
// quelli mancanti (o vuoti, 0 byte: equivalenti a mancanti). Non tocca mai
// i file già presenti e non scarica nulla dalla rete.
//
// Contratto:
//   * RestoreMissingManifestsAtStartup() è ASINCRONA: ritorna subito e fa la
//     I/O su un thread dedicato (nessun blocco dell'init di Steam).
//   * Una sola esecuzione per processo (guard atomico).
//   * Tutto è best-effort: errori di I/O solo loggati, mai fatali.
//   * No-op quando [manifest_cache] restore_on_startup è false (default true).
// ============================================================================

namespace ac::hooks::ManifestRestore {

// Scansione + ripristino async dei .manifest mancanti (vedi contratto sopra).
// Thread-safe, idempotente per processo.
void RestoreMissingManifestsAtStartup();

}  // namespace ac::hooks::ManifestRestore
