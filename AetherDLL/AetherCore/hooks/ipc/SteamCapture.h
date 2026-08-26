#pragma once

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Runtime helpers shared by the IPC handlers.
//
// Captures the SteamEngine pointer (via the AetherOnline GetAppIDForCurrentPipe
// hook), resolves the effective app id for the current pipe, and grows IPC
// response buffers using Steam's own CUtlBuffer::EnsureCapacity.
// ---------------------------------------------------------------------------
namespace ac::capture {

// Resolves CUtlBuffer::EnsureCapacity from the pattern table. Safe to call once
// during hook install; if unresolved, EnsureBufferSize degrades to a no-op grow.
void Init(HMODULE diversion);

// The app id Steam reports for the current pipe, or 0 if the engine pointer has
// not been captured yet.
steam::AppId GetAppIdForCurrentPipe();

// Effective app id for IPC handling: the AetherOnline real app id when a session
// is active, otherwise the current pipe's app id (resolved via SteamEngine).
steam::AppId CurrentRouteAppId();

// Grows pWrite to at least 'size' bytes and sets its put cursor to 'size'.
void EnsureBufferSize(steam::CUtlBuffer* pWrite, std::int32_t size);

// ---------------------------------------------------------------------------
// AetherOnline stats-scope.
//
// While an IClientUserStats IPC call is being dispatched (see IPCBus), the
// Steam client resolves the "current game" for stats operations through
// GetAppIDForCurrentPipe. Under -aetheronline that returns the Spacewar/480
// masquerade, so the client's stats subsystem would store/read stats for app
// 480: unlocks never reach the overlay or library and nothing persists for the
// real game. Bracketing the dispatch with EnterStatsScope/LeaveStatsScope lets
// GetAppIDForCurrentPipe resolve the real app id for the duration of the call
// only — every other path keeps the 480 identity (see AetherOnlineHooks).
//
// Depth-based so nested IClientUserStats dispatches on the same thread stay
// correctly bracketed (mirrors LumaCore's SetUserStatsContext).
// ---------------------------------------------------------------------------
void EnterStatsScope();
void LeaveStatsScope();

// True when this thread is currently inside an IClientUserStats dispatch.
bool IsStatsScopeActive();

}  // namespace ac::capture
