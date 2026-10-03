#pragma once

#include <mutex>

// ---------------------------------------------------------------------------
// core/HookBootstrap — orchestrazione esplicita dell'installazione hook (P8).
//
// Prima viveva tutto dentro hooks/steamui/SteamUIHook.cpp: batch steamclient,
// retry del redirect UI e retry dei pattern tardivi mescolati con il redirect
// stesso. Qui le fasi hanno nomi e responsabilità chiare:
//
//   fase 1  InstallSteamClientBatch  — kernel32 + steamclient + IPC + wire
//   fase 2  ArmSteamUiRedirect       — redirect LoadModuleWithPath (+ retry)
//   retry   StartPatternLateRetry    — tabelle pattern arrivate in ritardo
//
// I thread di retry sono worker nominati (workers::StartWorker), quindi
// shutdown, join e diagnostica passano tutti dall'infrastruttura workers.
// ---------------------------------------------------------------------------
namespace ac::bootstrap {

// Serializza i batch di registrazione hook (il batch steamclient e il
// redirect steamui possono venire rieseguiti in parallelo dai retry worker).
std::mutex& BatchMutex();

// Fase 1: registra e abilita i batch kernel32/steamclient. Idempotente: alla
// riesecuzione registra solo gli hook precedentemente mancati.
void InstallSteamClientBatch();

// Fase 2: arma il redirect steamui!LoadModuleWithPath il prima possibile; se
// steamui.dll non è ancora mappato avvia il worker di retry differito.
void ArmSteamUiRedirect();

// Fase 1+2: entry point di dllmain (step 9) e della riesecuzione del retry
// dei pattern tardivi.
void InstallAllHooks();

// Avvia il worker "pattern_late_retry": se qualche tabella pattern mancava
// all'avvio, riprova le sorgenti per una finestra limitata e reinstalla gli
// hook mancati in-session (no-op quando tutte le tabelle sono già presenti).
void StartPatternLateRetry();

// Chiede ai worker di retry di fermarsi senza attendere il join (il join
// avviene in workers::Shutdown). Sicura da chiamare più volte.
void RequestRetryStop();

}  // namespace ac::bootstrap
