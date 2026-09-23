#pragma once

#include <atomic>
#include <functional>
#include <string>

// ---------------------------------------------------------------------------
// core/Workers — background-work infrastructure (audit P3).
//
// Replaces every `std::thread(...).detach()` in AetherCore with work owned by
// a single registry, so that:
//   * one-shot async jobs go through the task queue (Submit);
//   * long-lived loops are named, stop-flag-aware workers (StartWorker);
//   * the explicit shutdown path can stop + join everything and LOG it.
//
// This is private service infrastructure (allowed module-local lifecycle per
// docs/ARCHITECTURE.md): it owns threads, not domain state.
// ---------------------------------------------------------------------------
namespace ac::workers {

// Enqueues a one-shot job. Returns false when the queue has been shut down
// (caller decides how to log). Jobs must be self-contained; exceptions are
// caught and logged, never propagated.
bool Submit(std::function<void()> job);

// Starts a named long-lived worker. `body` receives the stop flag: it must
// poll it (and use it in any wait predicate) so shutdown stays snappy.
// Completed workers are reaped automatically on the next StartWorker call.
// Returns false when workers are shut down (caller logs).
bool StartWorker(const std::string& name,
                 std::function<void(std::atomic<bool>& stop)> body);

// Stops everything: sets stop flags, drains and joins the task queue, joins
// every worker. Logs a full summary. Idempotent; NEVER call from DllMain
// (explicit off-loader-lock shutdown only).
void Shutdown();

// One-line registry summary for diagnostics/log lines.
std::string SummaryText();

}  // namespace ac::workers
