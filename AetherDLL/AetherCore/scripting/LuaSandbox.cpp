#include "pch.h"
#include "scripting/LuaSandbox.h"

extern "C" {
#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>
}

namespace ac::script::sandbox {
namespace {

struct StdLib {
    const char* name;
    lua_CFunction loader;
};

// Whitelist: provider scripts are data-only, so they need pure data
// primitives only (closures, string/table manipulation, math). Everything
// else that luaL_openlibs would open (io, os, package, debug, coroutine,
// utf8) stays closed:
//   * io/os      -> arbitrary file and shell access;
//   * package    -> package.loadlib() loads an arbitrary DLL into steam.exe;
//   * debug      -> registry introspection (would recover the closed io/os
//                   tables via package.loaded) and VM tampering;
//   * coroutine  -> unneeded here, and every extra surface is error surface.
constexpr StdLib kAllowedLibs[] = {
    {"_G",            luaopen_base},
    {LUA_TABLIBNAME,  luaopen_table},
    {LUA_STRLIBNAME,  luaopen_string},
    {LUA_MATHLIBNAME, luaopen_math},
};

// The base lib still hands scripts code loaders and GC control even though
// the loaders' backing libraries were never opened. dofile/loadfile/load/
// loadstring would execute external code pulled from disk or strings, and
// collectgarbage would let a script pause the GC and starve the host
// process of memory. "require" is listed defensively: it lives in the
// package lib (closed above), so nil-ing it is a no-op today and a guard
// against anyone reopening package without this knowledge.
constexpr const char* kRemovedGlobals[] = {
    "dofile",
    "loadfile",
    "load",
    "loadstring",
    "require",
    "collectgarbage",
};

}  // namespace

void Install(lua_State* L) {
    // Whitelist load instead of luaL_openlibs. luaL_requiref registers each
    // library under its name and leaves it on the stack (popped below).
    // Note: the loaded-libs registry is unreachable from scripts because the
    // package library itself is never opened, so these registrations cannot
    // be abused to recover closed libraries from Lua code.
    for (const StdLib& lib : kAllowedLibs) {
        luaL_requiref(L, lib.name, lib.loader, 1);
        lua_pop(L, 1);
    }

    // Strip the code-loading and GC-control sinks from the base lib.
    for (const char* name : kRemovedGlobals) {
        lua_pushnil(L);
        lua_setglobal(L, name);
    }
}

}  // namespace ac::script::sandbox
