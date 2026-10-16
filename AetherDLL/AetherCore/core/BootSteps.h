#pragma once

#include <memory>
#include <string>
#include <vector>

#include "core/Settings.h"

// ---------------------------------------------------------------------------
// BootSteps — one function per bootstrap stage (TNG-001 extraction).
//
// InitThreadLogic used to own all ten stages inline (~170 lines): paths,
// logging, diversion, patterns, ABI, Lua, hooks, safety nets and watchers.
// Each stage below does exactly one thing and returns success/failure, so
// InitThreadLogic is a straight-line orchestrator with early returns and a
// future maintainer can reorder or insert stages by moving lines, not by
// re-reading the whole pipeline. No threads, ordering or log bytes change.
// ---------------------------------------------------------------------------
namespace ac::boot {

struct BootContext {
    // Settings snapshot published by InitLoggingStage; carried (not
    // re-fetched) so all stages observe one consistent configuration.
    // Paths live in g_state (written by ResolveStage before use).
    std::shared_ptr<const Settings> settings;
    bool configValid = false;
    bool patternsOk = false;
};

// Paths + aethercore dir. False = module path unavailable/truncated: the
// caller must abort before any directory write (never operate on a
// truncated Steam prefix).
bool ResolveStage(HMODULE self, BootContext& ctx);

// Settings snapshot + logger + status writer + bridge report + pin.
// Always true (settings fall back to defaults; failures are logged).
bool InitLoggingStage(BootContext& ctx);

// Pin + build id + diversion + steamclient SHA. False = diversion failed:
// caller must publish status and abort.
bool InitDiversionStage();

// Pattern engine + IPC spec concurrently, hook-target selection, UI
// redirect arm. Always true (missing tables only degrade gracefully).
bool InitPatternsStage(BootContext& ctx);

// ABI table seed + build fallback. Always true, diagnostic only.
void SeedAbiStage();

// Lua scan. False only when the interpreter itself fails to initialise.
bool InitLuaStage();

// Steamclient hook batch + late-pattern retry. Always true.
void InstallHooksStage();

// Achievement backup + manifest restore sweep. Always true (best-effort).
void StartSafetyNetsStage();

// Lua + ACF watchers. Always true.
void StartWatchersStage(const BootContext& ctx);

// Full pipeline in dependency order (see the step list in the .cpp).
// Called once, off the loader lock, on the init thread.
void InitThreadLogic(HMODULE self);

} // namespace ac::boot
