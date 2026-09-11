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

}  // namespace ac::hooks::ManifestRestore
