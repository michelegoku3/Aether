//! Costanti di contratto condivise tra più moduli/tool.
//!
//! Unico posto dove vivono i suffissi di backup/rinomina usati sia dagli
//! external tool sia dall'engine online: una modifica si propaga da sola
//! (DRY) e i test fanno affidamento sugli stessi valori usati in produzione.

/// Suffisso dei backup degli eseguibili processati da Steamless
/// (es. `game.exe.steamstub.bak` — naming del tool Steamless stesso).
pub const STEAMLESS_BACKUP_SUFFIX: &str = ".steamstub.bak";

/// Suffisso dell'output di Steamless prima che sostituisca l'originale
/// (es. `game.exe.unpacked.exe`).
pub const STEAMLESS_UNPACKED_SUFFIX: &str = ".unpacked.exe";

/// Suffisso con cui UCOnline2 (patch.bat) neutralizza in modo reversibile
/// gli emulatori concorrenti (SteamFix/OFME di online-fix.me e i loro proxy).
pub const UCO_DISABLED_SUFFIX: &str = ".uco-disabled";

/// Soglia (byte) sotto cui un proxy DLL generico (`version.dll`, `dxgi.dll`,
/// ...) viene considerato parte di un emulatore concorrente e non un file
/// di gioco legittimo. Specchia la soglia di UCOnline2 patch.bat (300 KiB).
pub const UCO_PROXY_MAX_BYTES: u64 = 307_200;

/// True quando un nome file è un backup Steamless.
pub fn is_steamless_backup_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(STEAMLESS_BACKUP_SUFFIX)
}

/// True quando un nome file è un output Steamless non ancora applicato.
pub fn is_steamless_unpacked_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(STEAMLESS_UNPACKED_SUFFIX)
}
