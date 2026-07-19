#pragma once

#include <cstdint>
#include <optional>

// ---------------------------------------------------------------------------
// Runtime IPC method spec loader.
//
// Steam identifies IPC methods by a per-build hash (funcHash) that can change
// when Steam updates. AetherCore ships compile-time fallback hashes in
// Constants.h, but those only work for the Steam build they were extracted
// from. This module loads a per-build TOML (from the same KoriaPolis pattern
// repo) that maps qualified method names to their current funcHash values.
//
// Fallback: when no TOML exists (first run, offline, repo doesn't ship IPC
// specs yet), ResolveHash returns nullopt and IPCBus keeps using the hardcoded
// constants. No functionality is lost.
//
// This replaces LumaCore's IpcSpecLoader + IpcMethodLoader + IpcDispatch
// (~900 lines, own HTTP client, own SHA computation, own cache logic) with a
// single module (~120 lines) that reuses AetherCore's existing infrastructure.
// ---------------------------------------------------------------------------
namespace ac::ipcspec {

// Loads IPC spec for the current steamclient build. Uses the SHA-256 already
// computed in AetherCoreState. Must be called after pattern::Init() so the
// pattern cache directory exists. Safe to call multiple times — subsequent
// calls are no-ops when already loaded.
bool Init();

// Resolves the funcHash for a qualified method name like
// "IClientUser::GetSteamID". Returns nullopt when no spec is loaded or the
// method name is absent (caller falls back to the compile-time constant).
std::optional<std::uint32_t> ResolveHash(const char* qualifiedName);

// Whether a spec TOML was loaded and parsed successfully.
bool IsLoaded();

}  // namespace ac::ipcspec
