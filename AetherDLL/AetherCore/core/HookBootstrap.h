#pragma once

#include <mutex>

// ---------------------------------------------------------------------------
// core/HookBootstrap — explicit hook-installation orchestration (P8).
//
// Everything used to live inside hooks/steamui/SteamUIHook.cpp: steamclient
// batch, UI redirect retry and late-pattern retries mixed with the redirect
// itself. Here phases have clear names and responsibilities:
//
//   phase 1  InstallSteamClientBatch  — kernel32 + steamclient + IPC + wire
//   phase 2  ArmSteamUiRedirect       — redirect LoadModuleWithPath (+ retry)
//   retry    StartPatternLateRetry    — late-arriving pattern tables
//
// Retry threads are named workers (workers::StartWorker), so shutdown, join
// and diagnostics all flow through the workers infrastructure.
// ---------------------------------------------------------------------------
namespace ac::bootstrap {

// Serializes hook registration batches (the steamclient batch and the steamui
// redirect may be re-run in parallel by retry workers).
std::mutex& BatchMutex();

// Phase 1: register and enable the kernel32/steamclient batches. Idempotent: on
// re-run it registers only the previously missed hooks.
void InstallSteamClientBatch();

// Phase 2: arm the steamui!LoadModuleWithPath redirect ASAP; when steamui.dll
// is not mapped yet, start the deferred retry worker.
void ArmSteamUiRedirect();

// Phase 1+2: entry point of dllmain (step 9) and of the late-pattern retry
// re-run.
void InstallAllHooks();

// Start the "pattern_late_retry" worker: when some pattern table was missing
// at startup, retry the sources for a bounded window and reinstall the missed
// hooks in-session (no-op when every table is already present).
void StartPatternLateRetry();

// Ask the retry workers to stop without awaiting the join (the join happens in
// workers::Shutdown). Safe to call multiple times.
void RequestRetryStop();

}  // namespace ac::bootstrap
