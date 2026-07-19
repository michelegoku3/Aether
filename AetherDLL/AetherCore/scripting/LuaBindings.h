#pragma once

// ---------------------------------------------------------------------------
// Lua binding registration.
//
// Extracted from ScriptEngine.cpp so that domain-specific bindings
// (addappid, addtoken, etc.) live separately from the interpreter lifecycle
// (init, sandbox, file parsing). Each axis of change has its own file.
// ---------------------------------------------------------------------------

#include <cstddef>

struct lua_State;

namespace ac::script::bindings {

    // Per-file parse statistics. ScriptEngine creates one per file, passes it
    // via SetActiveStats, and reads it back for logging after the parse.
    struct ParseStats {
        std::size_t autoApps = 0;
        std::size_t depots = 0;
        std::size_t keyedDepots = 0;
        std::size_t accessTokens = 0;
        std::size_t manifestOverrides = 0;
        std::size_t appTickets = 0;
        std::size_t eTickets = 0;
        std::size_t eticketUrls = 0;
    };

    // The stdlib sandbox surface lives in scripting/LuaSandbox.{h,cpp}
    // (whitelist install, called internally by RegisterAll).

    // Makes global reads/writes case-insensitive so "AddAppId" == "addappid".
    void InstallCaseInsensitiveGlobals(lua_State* L);

    // Registers every AetherCore Lua binding (addappid, addtoken, setmanifestid,
    // setappticket, seteticket, seteticketurl, lchttpget) and installs sandbox +
    // case-insensitive globals. Called once during Init().
    void RegisterAll(lua_State* L);

    // Sets the active per-file stats pointer. Bindings increment counters here.
    // Pass nullptr to disable stats collection.
    void SetActiveStats(ParseStats* stats);

}  // namespace ac::script::bindings
