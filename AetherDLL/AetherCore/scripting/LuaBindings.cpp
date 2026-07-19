#include "pch.h"
// ---------------------------------------------------------------------------
// Lua binding registration.
//
// Every function exposed to .lua scripts lives here. The validation helpers
// (CheckAppId, CheckString, etc.) are file-local so they stay close to the
// bindings that use them. ScriptEngine.cpp owns the interpreter lifecycle;
// this file owns what the interpreter can call.
// ---------------------------------------------------------------------------

#include "pch.h"
#include "scripting/LuaBindings.h"

extern "C" {
#include <lauxlib.h>
#include <lua.h>
}

#include <cctype>
#include <cstdint>
#include <string>
#include <string_view>

#include "core/AetherCoreState.h"
#include "credentials/CredentialStore.h"
#include "credentials/HexCodec.h"
#include "scripting/LuaData.h"
#include "scripting/LuaSandbox.h"
#include "core/Logger.h"
#include "network/RuntimeHttp.h"

namespace ac::script::bindings {
    namespace {

        constexpr const char* kModule = "Lua";
        constexpr std::size_t kDepotKeyHexChars = 64;  // 32-byte keys as 64 hex chars

        // Per-file stats pointer, set by ScriptEngine before each file parse.
        ParseStats* s_activeStats = nullptr;

        // ---- Input validation helpers ----------------------------------------------
        // Every binding routes its arguments through these so validation is uniform and
        // malformed scripts fail with a clear Lua error instead of corrupting state.

        steam::AppId CheckAppId(lua_State* L, int idx, const char* where) {
            if (!lua_isinteger(L, idx)) {
                luaL_error(L, "%s: arg #%d must be an integer", where, idx);
            }
            lua_Integer v = lua_tointeger(L, idx);
            if (v < 0 || v > static_cast<lua_Integer>(UINT32_MAX)) {
                luaL_error(L, "%s: arg #%d out of uint32 range", where, idx);
            }
            return static_cast<steam::AppId>(v);
        }

        std::string_view CheckString(lua_State* L, int idx, const char* where) {
            if (!lua_isstring(L, idx)) {
                luaL_error(L, "%s: arg #%d must be a string", where, idx);
            }
            std::size_t len = 0;
            const char* p = lua_tolstring(L, idx, &len);
            return { p, len };
        }

        bool TryParseDecimalU64(std::string_view s, std::uint64_t& out) {
            if (s.empty()) return false;
            std::uint64_t value = 0;
            for (char c : s) {
                if (c < '0' || c > '9') return false;
                std::uint64_t digit = static_cast<std::uint64_t>(c - '0');
                if (value > (UINT64_MAX - digit) / 10) return false;  // overflow guard
                value = value * 10 + digit;
            }
            out = value;
            return true;
        }

        std::uint64_t CheckOptionalU64(lua_State* L, int idx, const char* where) {
            if (lua_isnoneornil(L, idx)) return 0;
            if (!lua_isinteger(L, idx)) {
                luaL_error(L, "%s: arg #%d must be an integer", where, idx);
                return 0;
            }
            lua_Integer v = lua_tointeger(L, idx);
            if (v < 0) {
                luaL_error(L, "%s: arg #%d must be >= 0", where, idx);
                return 0;
            }
            return static_cast<std::uint64_t>(v);
        }

        // ---- Bindings --------------------------------------------------------------

        // addappid(depotId [, _, hexKey])
        int L_AddAppId(lua_State* L) {
            steam::AppId depot = CheckAppId(L, 1, "addappid");
            std::string key;
            if (lua_gettop(L) > 2) {
                std::string_view raw = CheckString(L, 3, "addappid");
                if (!raw.empty()) {
                    if (raw.size() != kDepotKeyHexChars) {
                        return luaL_error(L, "addappid: key must be exactly 64 hex characters");
                    }
                    if (!ac::hex::Decode(raw)) return luaL_error(L, "addappid: key must be hex");
                    key.assign(raw.data(), raw.size());
                }
            }
            luadata::RecordDepot(depot, key);
            if (s_activeStats) {
                ++s_activeStats->depots;
                if (!key.empty()) ++s_activeStats->keyedDepots;
            }
            return 0;
        }

        // addtoken(appId, tokenDecimalString)
        int L_AddToken(lua_State* L) {
            steam::AppId app = CheckAppId(L, 1, "addtoken");
            std::string_view tok = CheckString(L, 2, "addtoken");
            std::uint64_t token = 0;
            if (!TryParseDecimalU64(tok, token)) {
                return luaL_error(L, "addtoken: token must be a decimal uint64");
            }
            luadata::SetAccessToken(app, token);
            if (s_activeStats) ++s_activeStats->accessTokens;
            return 0;
        }

        // setmanifestid(depotId, gidDecimalString [, size])
        int L_SetManifestId(lua_State* L) {
            steam::AppId depot = CheckAppId(L, 1, "setmanifestid");
            std::string_view gidStr = CheckString(L, 2, "setmanifestid");
            std::uint64_t gid = 0;
            if (!TryParseDecimalU64(gidStr, gid)) {
                return luaL_error(L, "setmanifestid: gid must be a decimal uint64");
            }
            std::uint64_t size = CheckOptionalU64(L, 3, "setmanifestid");
            luadata::SetManifestOverride(depot, { gid, size });
            if (s_activeStats) ++s_activeStats->manifestOverrides;
            return 0;
        }

        // setappticket(appId, hexTicket) — decoded and written to the registry.
        int L_SetAppTicket(lua_State* L) {
            steam::AppId app = CheckAppId(L, 1, "setappticket");
            auto decoded = ac::hex::Decode(CheckString(L, 2, "setappticket"));
            if (!decoded) return luaL_error(L, "setappticket: ticket must be hex");
            if (!ac::credential::WriteAppOwnershipTicket(app, *decoded)) {
                return luaL_error(L, "setappticket: registry write failed");
            }
            if (s_activeStats) ++s_activeStats->appTickets;
            return 0;
        }

        // seteticket(appId, hexTicket) — decoded and written to the registry.
        int L_SetETicket(lua_State* L) {
            steam::AppId app = CheckAppId(L, 1, "seteticket");
            auto decoded = ac::hex::Decode(CheckString(L, 2, "seteticket"));
            if (!decoded) return luaL_error(L, "seteticket: ticket must be hex");
            if (!ac::credential::WriteEncryptedTicket(app, *decoded)) {
                return luaL_error(L, "seteticket: registry write failed");
            }
            if (s_activeStats) ++s_activeStats->eTickets;
            return 0;
        }

        // lcHttpGet(url) -> body, status
        // Allowlisted HTTP GET so scripts can fetch a manifest gid without bouncing
        // through an external tool. Blocked hosts return ("", 403) so a script cannot
        // tell "gate refused" from "server refused" (data-exfil mitigation).
        int L_HttpGet(lua_State* L) {
            std::string_view url = CheckString(L, 1, "lcHttpGet");
            http::Response resp = http::Get(url);
            lua_pushlstring(L, resp.body.data(), resp.body.size());
            lua_pushinteger(L, static_cast<lua_Integer>(resp.status));
            return 2;
        }

        // seteticketurl(url)
        // Configures a Lua-driven backend URL used by the runtime ETicket mint path.
        // The actual POST still goes through RuntimeHttp's host allowlist gate, so a
        // hostile script cannot exfiltrate to an arbitrary backend unless the user
        // explicitly allowlists that host.
        int L_SetEticketUrl(lua_State* L) {
            std::string_view url = CheckString(L, 1, "seteticketurl");
            luadata::SetEticketUrl(std::string(url));
            if (s_activeStats) ++s_activeStats->eticketUrls;
            return 0;
        }

        // ---- Case-insensitive globals ----------------------------------------------

        std::string ToLower(const char* s) {
            std::string out(s);
            for (char& c : out) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
            return out;
        }

        int L_GlobalsIndex(lua_State* L) {
            lua_pushstring(L, ToLower(luaL_checkstring(L, 2)).c_str());
            lua_rawget(L, 1);
            return 1;
        }

        int L_GlobalsNewIndex(lua_State* L) {
            lua_pushstring(L, ToLower(luaL_checkstring(L, 2)).c_str());
            lua_pushvalue(L, 3);
            lua_rawset(L, 1);
            return 0;
        }

        // ---- Sandbox (implemented in LuaSandbox.cpp) ----------------------------

    }  // namespace (anonymous)

    // ---- Public interface ------------------------------------------------------

    void InstallCaseInsensitiveGlobals(lua_State* L) {
        lua_rawgeti(L, LUA_REGISTRYINDEX, LUA_RIDX_GLOBALS);
        lua_newtable(L);
        lua_pushcfunction(L, L_GlobalsIndex);
        lua_setfield(L, -2, "__index");
        lua_pushcfunction(L, L_GlobalsNewIndex);
        lua_setfield(L, -2, "__newindex");
        lua_setmetatable(L, -2);
        lua_pop(L, 1);
    }

    void RegisterAll(lua_State* L) {
        sandbox::Install(L);
        lua_register(L, "addappid", L_AddAppId);
        lua_register(L, "addtoken", L_AddToken);
        lua_register(L, "setmanifestid", L_SetManifestId);
        lua_register(L, "setappticket", L_SetAppTicket);
        lua_register(L, "seteticket", L_SetETicket);
        lua_register(L, "seteticketurl", L_SetEticketUrl);
        lua_register(L, "lchttpget", L_HttpGet);
        InstallCaseInsensitiveGlobals(L);
    }

    // Called by ScriptEngine before parsing each file.
    void SetActiveStats(ParseStats* stats) {
        s_activeStats = stats;
    }

}  // namespace ac::script::bindings
