//! Eventi per il refresh della libreria Aether.
//!
//! Un solo contratto, deterministico e senza dipendenze da filesystem:
//! ogni operazione in-app che installa (o rimuove) un `.lua` in
//! `<Steam>/config/stplug-in` — download store da hubcap/luatools/ryuu/moed,
//! install locale (singolo e bulk), rimozione dalla libreria — emette
//! `LUA_LIBRARY_EVENT` al momento del SUCCESSO. Il frontend (hook
//! `useLibraryGames`) risponde con una rescan in background.
//!
//! Niente watcher di directory: i cambi fatti a mano da Esplora File si
//! recuperano col pulsante Refresh (comportamento noto e prevedibile, zero
//! thread, zero costi a regime).

use tauri::{AppHandle, Emitter};

/// Evento "la libreria .lua è cambiata: rifai la scan" (vedi mod doc).
pub const LUA_LIBRARY_EVENT: &str = "library://lua-changed";

/// Da chiamare nei comandi, subito dopo che l'installazione/rimozione del
/// `.lua` è andata a buon fine. Fire-and-forget: l'emit non può fallire in
/// modo significativo (payload vuoto, listener in locale).
pub fn notify_lua_changed(app: &AppHandle) {
    let _ = app.emit(LUA_LIBRARY_EVENT, ());
}
