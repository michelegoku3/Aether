#pragma once

#include "framework.h"

// ---------------------------------------------------------------------------
// SteamUIHook — SOLO il redirect del caricamento di steamclient64.dll (P8).
//
// L'orchestrazione (batch steamclient, retry differito, retry dei pattern
// tardivi) vive in core/HookBootstrap. Questo modulo fa una sola cosa: in copy
// mode, quando steamui.dll carica steamclient64.dll, la chiamata viene
// deviata su acoverlay.dll; in live mode è un no-op.
// ---------------------------------------------------------------------------
namespace ac::hooks::steamui {

// Installa l'hook su steamui!LoadModuleWithPath. Ritorna true solo quando il
// redirect è effettivamente attivo oppure non serve (live mode): una DLL
// steamui mappata NON basta (pattern mancante o MinHook fallito tengono vivo
// il retry del bootstrap). Thread-safe: prende il mutex di batch del
// bootstrap perché può correre in parallelo alla riesecuzione del batch.
bool InstallSteamUiRedirect();

}  // namespace ac::hooks::steamui
