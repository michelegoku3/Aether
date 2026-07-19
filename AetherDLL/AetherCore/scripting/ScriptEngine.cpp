#include "pch.h"
// ---------------------------------------------------------------------------
// Lua interpreter lifecycle.
//
// Owns the lua_State, the serialisation mutex, file parsing, directory
// scanning, and the public Init/ParseFile/UnloadFile/Shutdown API.
//
// Domain-specific bindings (addappid, addtoken, etc.) live in LuaBindings.cpp.
// This file calls bindings::RegisterAll() once during Init() to arm them.
// ---------------------------------------------------------------------------

#include "pch.h"
#include "scripting/ScriptEngine.h"

extern "C" {
#include <lauxlib.h>
#include <lua.h>
}

#include <cstdint>
#include <filesystem>
#include <mutex>
#include <string>

#include "core/AetherCoreState.h"
#include "scripting/LuaBindings.h"
#include "scripting/LuaData.h"
#include "core/Logger.h"

namespace fs = std::filesystem;

namespace ac::script {
    namespace {

        constexpr const char* kModule = "Lua";

        lua_State* s_lua = nullptr;

        // Serialises interpreter use: Init runs on the init thread, ParseFile/UnloadFile
        // run on the watcher thread. One lua_State cannot be used concurrently.
        std::mutex s_luaMutex;

        // Filename like "480.lua" implicitly registers app 480 even if the script omits
        // addappid. Recorded under the current parse file for ref-counting.
        void AutoRegisterFromFilename(const fs::path& file) {
            const std::string stem = file.stem().string();
            if (stem.empty()) return;
            for (char c : stem) {
                if (c < '0' || c > '9') return;
            }
            try {
                unsigned long val = std::stoul(stem);
                if (val > 0 && val <= UINT32_MAX) {
                    const auto appId = static_cast<steam::AppId>(val);
                    luadata::RecordDepot(appId, "");
                    luadata::RecordLibraryApp(appId);
                    // Auto-apps are counted via the stats pointer set by ParseFileLocked.
                }
            }
            catch (...) {
            }
        }

        // Parses one file inside a BeginFile/EndFile bracket. Caller holds s_luaMutex.
        void ParseFileLocked(const fs::path& file) {
            const std::string path = file.string();
            // A re-parse first releases the file's old references.
            luadata::UnloadFile(path);

            bindings::ParseStats stats;
            bindings::SetActiveStats(&stats);
            luadata::BeginFile(path);
            AutoRegisterFromFilename(file);
            if (luaL_dofile(s_lua, path.c_str()) != LUA_OK) {
                const char* err = lua_tostring(s_lua, -1);
                AC_LOG_ERROR(kModule, "Error in %s: %s", path.c_str(), err ? err : "unknown");
                diag::Record("lua_parse_error", path);
                lua_pop(s_lua, 1);
            }
            else {
                AC_LOG_INFO(kModule,
                    "Loaded %s (autoApps=%zu, explicitDepots=%zu, keyed=%zu, tokens=%zu, manifests=%zu, appTickets=%zu, eTickets=%zu, eticketUrls=%zu).",
                    path.c_str(), stats.autoApps, stats.depots, stats.keyedDepots,
                    stats.accessTokens, stats.manifestOverrides, stats.appTickets, stats.eTickets,
                    stats.eticketUrls);
                diag::Record("lua_loaded", path);
            }
            luadata::EndFile();
            bindings::SetActiveStats(nullptr);
        }

        void ScanDirectory(const std::string& dir) {
            std::error_code ec;
            if (!fs::is_directory(dir, ec)) return;
            for (const auto& entry : fs::directory_iterator(dir, ec)) {
                if (entry.is_regular_file() && entry.path().extension() == ".lua") {
                    ParseFileLocked(entry.path());
                }
            }
        }

    }  // namespace

    bool Init() {
        AC_LOG_INFO(kModule, "Initialising script engine.");

        g_state.luaDir = g_state.steamInstallPath + "\\config\\stplug-in";
        CreateDirectoryA((g_state.steamInstallPath + "\\config").c_str(), nullptr);
        CreateDirectoryA(g_state.luaDir.c_str(), nullptr);

        std::lock_guard<std::mutex> lock(s_luaMutex);
        s_lua = luaL_newstate();
        if (!s_lua) {
            AC_LOG_ERROR(kModule, "Could not create Lua state.");
            return false;
        }

        bindings::RegisterAll(s_lua);

        ScanDirectory(g_state.luaDir);
        for (const std::string& extra : g_state.settings.luaExtraPaths) {
            ScanDirectory(extra);
        }

        // Files present at boot are baseline state, not hot-reload additions.
        // If the initial RecordDepot() calls stayed queued, the first later
        // NotifyLicenseChanged() could inject those appids a second time.
        luadata::ClearPendingChanges();

        AC_LOG_INFO(kModule, "Init complete. Configured depots: %zu", luadata::AllDepotIds().size());
        return true;
    }

    void ParseFile(const std::string& path) {
        std::lock_guard<std::mutex> lock(s_luaMutex);
        if (!s_lua) return;
        ParseFileLocked(fs::path(path));
    }

    void UnloadFile(const std::string& path) {
        // No interpreter call needed; just release the file's references. Still take
        // the lua mutex to serialise with parses touching the same data layer.
        std::lock_guard<std::mutex> lock(s_luaMutex);
        luadata::UnloadFile(path);
    }

    void Shutdown() {
        std::lock_guard<std::mutex> lock(s_luaMutex);
        if (s_lua) {
            lua_close(s_lua);
            s_lua = nullptr;
            AC_LOG_INFO(kModule, "Lua state closed.");
        }
    }

}  // namespace ac::script
