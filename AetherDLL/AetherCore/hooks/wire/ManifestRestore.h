#pragma once

// ============================================================================
// ManifestRestore — backup/restore dei manifest Steam in AetherData.
//
//   <AetherData>\backup\<app_id>\lua\*.manifest
//
// All'avvio di Steam il modulo prima copia in AetherData i manifest ancora
// presenti in Steam\depotcache e poi ripristina quelli mancanti. DirWatch
// sorveglia inoltre Steam\steamapps\appmanifest_*.acf: dopo la rimozione di
// un ACF (disinstallazione) richiama il ripristino mirato dell'app.
// ============================================================================

#include <cstdint>

namespace ac::hooks::ManifestRestore {

// Scansione + backup/restore async dei manifest all'avvio, una volta per
// processo. Best-effort e non bloccante per l'inizializzazione di Steam.
void RestoreMissingManifestsAtStartup();

// Ripristina in modo sincrono i manifest dell'app indicata dopo la rimozione
// di appmanifest_<app_id>.acf. È chiamato dal thread DirWatch dopo il debounce.
void RestoreMissingManifestsForApp(std::uint32_t appId);

// Archivia in AetherData un manifest appena generato da Hubcap
// (backup/<app_id>/lua/<depot>_<gid>.manifest) per ogni app il cui Lua lo
// referenzia (pin attivi o commentati). Chiamato dal worker di generazione
// subito dopo l'install in depotcache: il backup di startup coprirebbe il
// file solo al prossimo avvio di Steam, e AetherDesk potrebbe non essere in
// esecuzione. Best-effort: segue il contratto condiviso di copia (destinazione
// non vuota mai sovrascritta, copia atomica e verificata) e non lancia mai.
void BackupManifestAfterGeneration(std::uint32_t depotId, std::uint64_t gid);

}  // namespace ac::hooks::ManifestRestore
