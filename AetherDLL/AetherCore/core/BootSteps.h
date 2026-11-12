#pragma once

#include <memory>
#include <string>
#include <vector>

#include "core/Settings.h"

// ---------------------------------------------------------------------------
// BootSteps — one function per bootstrap stage.
//
// Stages: paths, logging, steamclient attach, patterns, ABI, Lua, hooks, safety
// nets and watchers. Each stage does exactly one thing and returns success/failure,
// so InitThreadLogic is a straight-line orchestrator with early returns and a
// maintainer can reorder or insert stages by moving lines, not by re-reading
// the whole pipeline. Step order and log output must not change.
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

// Pin + build id + steamclient path/SHA. Attaching to the module happens later,
// when the hook batch runs (Steam maps the client itself; loading it here would
// crash steam.exe).
void InitSteamclientStage();

// Pattern engine + IPC spec concurrently. Always true (missing tables only
// degrade gracefully).
bool InitPatternsStage(BootContext& ctx);

// ABI table seed + build fallback. Always true, diagnostic only.
void SeedAbiStage();

// Lua scan. False only when the interpreter itself fails to initialise.
bool InitLuaStage();

// Steamclient hook batch + late-pattern retry. Always true.
void InstallHooksStage();

// Achievement backup, manifest restore, owner policy, depot repair and donor probe.
// Always true (best-effort).
void StartSafetyNetsStage();

// Lua + ACF watchers. Always true.
void StartWatchersStage(const BootContext& ctx);

// Full pipeline in dependency order (see the step list in the .cpp).
// Called once, off the loader lock, on the init thread.
void InitThreadLogic(HMODULE self);

} // namespace ac::boot
