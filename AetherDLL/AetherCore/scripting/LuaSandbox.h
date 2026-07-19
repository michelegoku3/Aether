#pragma once

// ---------------------------------------------------------------------------
// Lua sandbox — standard-library surface for stplug-in scripts.
//
// The .lua files in Steam\config\stplug-in are third-party content executed
// inside steam.exe with full user privileges and in-process access to AetherCore
// internals, so the interpreter must expose the minimum surface that data-only
// provider scripts actually need — and nothing else.
//
// Install() uses an explicit WHITELIST (base/table/string/math) instead of
// opening everything and blacklisting known-dangerous names. A blacklist
// decays with time: it depends on the author remembering every escape hatch,
// and future Lua upgrades can add new ones silently. The whitelist fails
// closed by construction and mirrors the policy LumaCore already validated
// against this same script ecosystem.
//
// This module deliberately depends on nothing but the Lua C API so the
// security boundary stays inspectable and unit-testable in isolation.
// ---------------------------------------------------------------------------

struct lua_State;

namespace ac::script::sandbox {

    // Opens ONLY the whitelisted stdlib libraries in L and nils out the
    // code-loading sinks that the base library still provides. Call once,
    // right after luaL_newstate(), before registering any binding.
    // Scripts loaded afterwards cannot reach io/os/package/debug/coroutine.
    void Install(lua_State* L);

}  // namespace ac::script::sandbox
