#include "pch.h"
#include "utils/GameNameResolver.h"

#include <MinHook.h>

#include <algorithm>
#include <cctype>
#include <mutex>
#include <limits>
#include <string>
#include <unordered_map>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "utils/PatternEngine.h"

namespace ac::gamename {
namespace {

constexpr const char* kModule = "GameName";

std::string Fold(const std::string& in) {
    std::string out;
    out.reserve(in.size());
    for (unsigned char c : in) {
        if (std::isspace(c)) continue;  // titles differ in spacing across locales
        out.push_back(static_cast<char>(std::tolower(c)));
    }
    return out;
}

// int64 CAppInfoCache::GetAppDataFromAppInfo(this, appId, key, out, outSize)
using GetAppDataFromAppInfo_t = std::int64_t (*)(void*, steam::AppId, const char*,
                                                 std::uint8_t*, std::int32_t);

// Init-time plumbing (trampoline + double-init guard): write-once, read-only
// after Init — admitted module-local per the AetherCoreState.h rule.
// The shared runtime state (captured object + name cache) lives in
// g_state.gameName.
GetAppDataFromAppInfo_t o_GetAppDataFromAppInfo = nullptr;
bool s_hookCreated = false;

std::int64_t h_GetAppDataFromAppInfo(void* self, steam::AppId appId, const char* key,
                                     std::uint8_t* out, std::int32_t outSize) {
    // One-shot atomic capture of the CAppInfoCache instance (RCX / first arg).
    // CAS-guaranteed exactly-once: no separate "already logged" flag needed.
    void* prev = nullptr;
    if (self && g_state.gameName.appInfoCacheObj.compare_exchange_strong(prev, self)) {
        AC_LOG_INFO(kModule, "Captured CAppInfoCache 0x%p.", self);
        diag::Record("gamename_ready", "captured");
    }
    return o_GetAppDataFromAppInfo ? o_GetAppDataFromAppInfo(self, appId, key, out, outSize)
                                   : 0;
}

#if defined(_MSC_VER)
// Sentinel: "the raw call raised a SEH exception".
constexpr std::int64_t kProbeFault = std::numeric_limits<std::int64_t>::min();

// Dtor-free by contract: MSVC forbids __try/__except in functions that require
// C++ object unwinding (C2712), so the guard lives in this small helper.
// MEASURED (12:57 flightrec log): probing GetAppDataFromAppInfo for an appid
// with NO local AppInfo record faults inside steamclient and kills the process
// before any flush; a guarded probe degrades that to "name unknown".
std::int64_t ProbeAppInfoGuarded(void* cache, steam::AppId appId,
                                 std::uint8_t* buf, std::int32_t bufSize) {
    __try {
        return o_GetAppDataFromAppInfo(cache, appId, "common/name", buf, bufSize);
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        return kProbeFault;
    }
}
#endif

}  // namespace

void Init(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_WARN(kModule, "No diversion module; name resolver disabled.");
        return;
    }
    if (s_hookCreated) return;

    void* target = pattern::ResolveAddress("GetAppDataFromAppInfo", "steamclient", diversion);
    if (!target) {
        AC_LOG_WARN_ONCE(kModule,
                         "GetAppDataFromAppInfo unresolved; game titles stay empty "
                         "(presence still works with app ids only).");
        diag::Record("gamename_miss", "pattern");
        return;
    }

    // MH_Initialize is also done by HookManager::InstallAll; tolerate already-init.
    MH_STATUS init = MH_Initialize();
    if (init != MH_OK && init != MH_ERROR_ALREADY_INITIALIZED) {
        AC_LOG_ERROR(kModule, "MH_Initialize failed: %s", MH_StatusToString(init));
        return;
    }

    MH_STATUS st = MH_CreateHook(target, reinterpret_cast<void*>(h_GetAppDataFromAppInfo),
                                 reinterpret_cast<void**>(&o_GetAppDataFromAppInfo));
    if (st != MH_OK && st != MH_ERROR_ALREADY_CREATED) {
        AC_LOG_ERROR(kModule, "CreateHook GetAppDataFromAppInfo failed: %s",
                     MH_StatusToString(st));
        diag::Record("gamename_hook_fail", MH_StatusToString(st));
        return;
    }
    st = MH_EnableHook(target);
    if (st != MH_OK && st != MH_ERROR_ENABLED) {
        AC_LOG_ERROR(kModule, "EnableHook GetAppDataFromAppInfo failed: %s",
                     MH_StatusToString(st));
        return;
    }
    s_hookCreated = true;
    AC_LOG_INFO(kModule, "GetAppDataFromAppInfo hook armed at 0x%p.", target);
}

bool Ready() {
    return g_state.gameName.appInfoCacheObj.load() != nullptr && o_GetAppDataFromAppInfo != nullptr;
}

std::string ForApp(steam::AppId appId) {
    if (appId == 0) return {};

    // 1. Cache hit (positive o negative)
    if (auto cached = g_state.gameName.nameCache.Get(appId)) {
        return *cached;  // Può essere "" (negative) o nome reale
    }

    // 2. Cache miss → query Steam
    std::string name;
    if (Ready()) {
        char buf[256] = {};
        // "common/name" triggers Steam's auto-localization path.
        std::int64_t len = 0;
#if defined(_MSC_VER)
        len = ProbeAppInfoGuarded(g_state.gameName.appInfoCacheObj.load(), appId,
                                  reinterpret_cast<std::uint8_t*>(buf),
                                  static_cast<std::int32_t>(sizeof(buf)));
        if (len == kProbeFault) {
            AC_LOG_WARN(kModule, "appinfo probe for app %u faulted (SEH); name unknown.",
                        appId);
            diag::Record("gamename_probe_fault", std::to_string(appId));
            g_state.gameName.nameCache.PutNegative(appId);
            return {};
        }
#else
        len = o_GetAppDataFromAppInfo(
            g_state.gameName.appInfoCacheObj.load(), appId, "common/name",
            reinterpret_cast<std::uint8_t*>(buf), static_cast<std::int32_t>(sizeof(buf)));
#endif
        // Return value is written size including trailing NUL when successful.
        if (len > 1) {
            name.assign(buf, static_cast<std::size_t>(len - 1));
        }
    }

    // 3. Memorizza (positive o negative)
    if (name.empty()) {
        g_state.gameName.nameCache.PutNegative(appId);
    } else {
        g_state.gameName.nameCache.Put(appId, name);
        AC_LOG_DEBUG_ONCE(kModule, "App %u name='%s'.", appId, name.c_str());
    }
    return name;
}

steam::AppId ResolveAppIdByName(const std::string& name) {
    if (name.empty()) return 0;

    const std::string key = Fold(name);
    if (key.empty()) return 0;

    // Cache holds negatives (0) too: a miss must not rescan the library on
    // every inbound PersonaState frame.
    static std::mutex s_mutex;
    static std::unordered_map<std::string, steam::AppId> s_cache;
    {
        std::lock_guard<std::mutex> lock(s_mutex);
        const auto it = s_cache.find(key);
        if (it != s_cache.end()) return it->second;
    }

    steam::AppId found = 0;
    for (const steam::AppId app : luadata::LibraryAppIds()) {
        if (app == 0) continue;
        const std::string candidate = ForApp(app);  // memoised per app id
        if (!candidate.empty() && Fold(candidate) == key) {
            found = app;
            break;
        }
    }

    {
        std::lock_guard<std::mutex> lock(s_mutex);
        s_cache[key] = found;
    }
    if (found != 0) {
        AC_LOG_INFO(kModule, "Reverse lookup '%s' -> app %u.", name.c_str(), found);
    } else {
        AC_LOG_DEBUG_ONCE(kModule, "Reverse lookup '%s': no configured app matches.",
                          name.c_str());
    }
    return found;
}

}  // namespace ac::gamename
