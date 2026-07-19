#pragma once

// ---------------------------------------------------------------------------
// CreateProcessHooks — pre-entry payload injection for OnlineFix games.
//
// Hooks kernel32.dll!CreateProcessW and CreateProcessAsUserW so that, when
// Steam spawns a game process under -onlinefix, the EOS bridge payload is
// injected BEFORE the child's entry point runs (via CREATE_SUSPENDED).
//
// This fixes the late-injection problem described in the architectural audit
// §3.4: PipeWatch-triggered injection arrives after EOS SDK may have already
// initialised, so the EOS hooks have no effect.  Pre-entry injection ensures
// the payload is loaded before any game code executes.
//
// The existing PipeWatch MaybeInject path remains as a safety net for any
// process that slips past the CreateProcess hooks; PID deduplication in
// AetherCoreState::onlinePayloadInjectedPids prevents double injection.
// ---------------------------------------------------------------------------
namespace ac::hooks {

// Registers detours on kernel32.dll!CreateProcessW and CreateProcessAsUserW.
// Safe to call during InstallAllHooks() — the hooks are passive until an
// OnlineFix session sets g_state.onlineFixRealAppId.
//
// Uses GetProcAddress directly (not PatternEngine) because these are stable
// kernel32 exports that never move.
void RegisterCreateProcessHooks();

}  // namespace ac::hooks
