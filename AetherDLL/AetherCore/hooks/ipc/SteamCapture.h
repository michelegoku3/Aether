#pragma once

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Runtime helpers shared by the IPC handlers.
//
// Captures the SteamEngine pointer (via the OnlineFix GetAppIDForCurrentPipe
// hook), resolves the effective app id for the current pipe, and grows IPC
// response buffers using Steam's own CUtlBuffer::EnsureCapacity.
//
// NOTE: LumaCore's RuntimeCapture also held the UserStats "stats scope" gate
// used purely by the achievement spoofer. That machinery is excluded here
// because all achievement code is out of scope for this project.
// ---------------------------------------------------------------------------
namespace ac::capture {

// Resolves CUtlBuffer::EnsureCapacity from the pattern table. Safe to call once
// during hook install; if unresolved, EnsureBufferSize degrades to a no-op grow.
void Init(HMODULE diversion);

// The app id Steam reports for the current pipe, or 0 if the engine pointer has
// not been captured yet.
steam::AppId GetAppIdForCurrentPipe();

// Effective app id for IPC handling: the OnlineFix real app id when a session
// is active, otherwise the current pipe's app id (resolved via SteamEngine).
steam::AppId CurrentRouteAppId();

// Grows pWrite to at least 'size' bytes and sets its put cursor to 'size'.
void EnsureBufferSize(steam::CUtlBuffer* pWrite, std::int32_t size);

}  // namespace ac::capture
