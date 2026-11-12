#pragma once

#include <mutex>

// ---------------------------------------------------------------------------
// core/HookBootstrap — hook-installation orchestration.
//
// Phases have clear names and responsibilities:
//
//   install   InstallSteamClientBatch  — kernel32 + steamclient + IPC + wire
//   retry     StartPatternLateRetry    — late-arriving pattern tables
//
// Retry threads are named workers (workers::StartWorker), so shutdown, join
// and diagnostics all flow through the workers infrastructure.
// ---------------------------------------------------------------------------
namespace ac::bootstrap {

// Serializes hook registration batches (the batch may be re-run by the retry
// worker).
std::mutex& BatchMutex();

// Register and enable the kernel32/steamclient batches. Idempotent: on re-run it
// registers only the previously missed hooks.
void InstallSteamClientBatch();

// Entry point of dllmain (step 9) and of the late-pattern retry re-run.
void InstallAllHooks();

// Start the "pattern_late_retry" worker: when some pattern table was missing
// at startup, retry the sources for a bounded window and reinstall the missed
// hooks in-session (no-op when every table is already present).
void StartPatternLateRetry();

// Register and enable the steamui batch (app overview). Idempotent; no-op while
// the module is not mapped or its pattern table is missing.
void InstallSteamUiBatch();

// Start the "steamui_attach" worker: steamui can map after the client, so the
// UI batch installs as soon as it appears, within a bounded window. The boot
// path never blocks on it.
void StartSteamUiAttachRetry();

// Ask the retry workers to stop without awaiting the join (the join happens in
// workers::Shutdown). Safe to call multiple times.
void RequestRetryStop();

} // namespace ac::bootstrap
