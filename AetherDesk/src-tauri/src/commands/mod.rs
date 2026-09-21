//! Moduli dei comandi Tauri + contesto di richiesta risolto lato backend.
//!
//! ## Il percorso Steam non è un parametro del client
//!
//! Fino a qui 22 comandi ricevevano `steam_path: String` dal frontend. Costi
//! reali di quel contratto:
//!
//! 1. **Latenza**: per ottenerlo la UI chiamava `get_settings`, che rilegge il
//!    file di configurazione e **decifra il keystore DPAPI** ad ogni colpo —
//!    una decifratura per popup, non per sessione.
//! 2. **Superficie d'attacco/bug**: un client poteva passare un percorso
//!    diverso da quello configurato, e ogni comando doveva ri-validarlo.
//! 3. **Deriva**: 22 firme da tenere allineate a mano con 30+ call site TS.
//!
//! Ora il percorso è risolto dal backend (unica fonte di verità: le
//! impostazioni). I comandi usano [`command_steam_path`] (fallisce con un
//! messaggio utente leggibile) o [`configured_steam_path`] (degrada con garbo
//! quando il percorso manca o Steam non è raggiungibile).

pub mod aether_desk;
pub mod aether_dll;
pub mod antivirus;
pub mod crack;
pub mod custom_css;
pub mod fs;
pub mod home_links;
pub mod game_info;
pub mod library;
pub mod local;
pub mod logs;
pub mod manifests;
pub mod monitor;
pub mod online;
pub mod versioning;
pub mod settings;
pub mod steam;
pub mod steamless;
pub mod store;
pub mod window;
pub mod workshop;

use crate::core::settings::require_steam_path;
use crate::util::validation::validate_steam_path;

/// Percorso Steam configurato, normalizzato e validato sul filesystem.
///
/// Sostituisce il vecchio parametro `steam_path` dei comandi: stesso errore
/// leggibile di prima quando il percorso manca (`require_steam_path`) e stessa
/// validazione rigorosa quando c'è (`validate_steam_path`), ma senza che il client
/// debba leggerlo né possa alterarlo.
///
/// Usare questa quando il comando **non può** lavorare senza un percorso Steam
/// valido (install/uninstall, scrittura manifest, download).
pub fn command_steam_path(app: &tauri::AppHandle) -> Result<String, String> {
    let steam_path = require_steam_path(app)?;
    validate_steam_path(&steam_path)?;
    Ok(steam_path)
}

/// Percorso Steam configurato **senza** validazione del filesystem.
///
/// Per i comandi che devono degradare con garbo invece di restituire un errore:
/// lettura versione DLL, check aggiornamenti, probe dei residui. Con `None`
/// (non configurato) o con un percorso irraggiungibile il chiamante risponde
/// "N/A" / 0 / nessun aggiornamento, esattamente come faceva prima quando il
/// client gli passava una stringa vuota o stale.
pub fn configured_steam_path(app: &tauri::AppHandle) -> Option<String> {
    require_steam_path(app).ok()
}
