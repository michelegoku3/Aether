#pragma once

#include <cstdint>
#include <optional>

// ---------------------------------------------------------------------------
// Runtime IPC method spec loader.
//
// Steam identifies IPC methods by a per-build hash (funcHash) that can change
// when Steam updates. AetherCore ships compile-time fallback hashes in
// Constants.h, but those only work for the Steam build they were extracted
// from. This module loads a per-build TOML (from the same priority-ordered
// pattern source registry as the pattern tables) that maps qualified method
// names to their current funcHash values.
//
// Fallback: when no TOML exists (first run, offline, repo doesn't ship IPC
// specs yet), the IPCBus keeps using the hardcoded constants. When a TOML is
// active but omits a method/interface, that handler is disabled rather than
// using a hash from a different Steam build.
//
// This replaces LumaCore's IpcSpecLoader + IpcMethodLoader + IpcDispatch
// (~900 lines, own HTTP client, own SHA computation, own cache logic) with a
// single module (~120 lines) that reuses AetherCore's existing infrastructure.
// ---------------------------------------------------------------------------
namespace ac::ipcspec {

// Per-method metadata carried by the spec TOML. funcHash is required;
// fencepost and argc are optional (0 = absent in the file). They are parsed
// for schema compatibility with the shared pattern repo and used only for
// diagnostics — never to gate dispatch.
struct MethodSpec {
    std::uint32_t hash = 0;
    std::uint32_t fencepost = 0;
    std::uint32_t argc = 0;
};

// Loads IPC spec for the current steamclient build. Uses the SHA-256 already
// computed in AetherCoreState. Must be called after pattern::Init() so the
// pattern cache directory exists. Safe to call multiple times — subsequent
// calls are no-ops when already loaded.
bool Init();

// Resolves the interface id for a name like "IClientUser". Returns nullopt
// when no spec is loaded or the interface is absent.
std::optional<std::uint8_t> ResolveInterfaceId(const char* interfaceName);

// Resolves the funcHash for a qualified method name like
// "IClientUser::GetSteamID". Returns nullopt when no spec is loaded or the
// method name is absent. Kept for callers that only need the hash.
std::optional<std::uint32_t> ResolveHash(const char* qualifiedName);

// Resolves the full per-method metadata (hash + optional fencepost/argc).
// Returns nullopt when no spec is loaded or the method name is absent.
std::optional<MethodSpec> ResolveMethodSpec(const char* qualifiedName);

// Whether a spec TOML was loaded and parsed successfully.
bool IsLoaded();

}  // namespace ac::ipcspec
