//! Steam process monitor: un unico poller condiviso che mantiene lo stato
//! "Steam in esecuzione" in memoria e notifica la UI SOLO ai cambiamenti.
//!
//! Design: la scansione del processo (sysinfo) è relativamente costosa, quindi
//! NON deve avvenire a ogni render/click ("is it running?" → O(1) read).
//! Un thread dedicato la esegue ogni `POLL_INTERVAL`; lo stato vive in una
//! `AtomicBool` globale letta da comandi e watcher indipendenti. Quando lo
//! stato cambia viene emesso l'evento tauri `steam://runtime-state` con il
//! nuovo booleano (payload `bool`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// Nome evento emesso ai listener della UI ad ogni transizione di stato.
pub const STEAM_RUNTIME_EVENT: &str = "steam://runtime-state";

/// Intervallo del poller: reattivo ai cambi ma trascurabile come carico.
const POLL_INTERVAL: Duration = Duration::from_millis(2500);

static STEAM_RUNNING: AtomicBool = AtomicBool::new(false);
static MONITOR_STARTED: AtomicBool = AtomicBool::new(false);

/// True se l'ultima scansione del monitor ha trovato Steam in esecuzione.
/// Lettura O(1): non esegue nessuna scansione (quella è del thread poller).
pub fn is_steam_running() -> bool {
    STEAM_RUNNING.load(Ordering::Relaxed)
}

/// Aggiornamento opportunistico dallo stesso chiamante di restart_steam:
/// dopo un kill o uno spawn riuscito la UI può reagire subito, il poller
/// correggerà eventuali errori alla prossima scansione.
pub fn mark(running: bool) {
    STEAM_RUNNING.store(running, Ordering::Relaxed);
}

fn scan_running(sys: &sysinfo::System) -> bool {
    sys.processes().values().any(|p| {
        let name = p.name().to_lowercase();
        name == "steam.exe" || name == "steam"
    })
}

/// Avvia il poller (idempotente: chiamate ulteriori sono no-op) ed esegue SUBITO
/// una prima scansione sincrona così lo stato è corretto ancora prima del
/// primo tick — niente finestra di "loading" nella UI dopo l'apertura.
pub fn start(app: AppHandle) {
    if MONITOR_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    std::thread::Builder::new()
        .name("steam-monitor".to_string())
        .spawn(move || {
            crate::desk_log_info!(
                "lifecycle",
                "Steam process monitor started (poll every {:?})",
                POLL_INTERVAL
            );
            let mut sys = sysinfo::System::new_all();
            let mut first = true;
            loop {
                sys.refresh_processes();
                let running = scan_running(&sys);
                let prev = STEAM_RUNNING.swap(running, Ordering::Relaxed);
                if running != prev || first {
                    // Primo tick: emetti comunque, così la UI converge subito
                    // anche se lo stato iniziale coincide col default.
                    if !first {
                        crate::desk_log_info!(
                            "lifecycle",
                            "Steam process state changed: {} -> {}",
                            if prev { "running" } else { "stopped" },
                            if running { "running" } else { "stopped" }
                        );
                    }
                    let _ = handle.emit(STEAM_RUNTIME_EVENT, running);
                    first = false;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        })
        .expect("failed to spawn steam-monitor thread");
}
