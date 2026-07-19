#include "pch.h"
#include "utils/GameNameResolver.h"

#include <MinHook.h>

#include <mutex>
#include <string>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/PatternEngine.h"

namespace ac::gamename {
namespace {

constexpr const char* kModule = "GameName";

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

    {
        std::lock_guard<std::mutex> lock(g_state.gameName.cacheMutex);
        auto it = g_state.gameName.nameCache.find(appId);
        if (it != g_state.gameName.nameCache.end()) return it->second;
    }

    std::string name;
    if (Ready()) {
        char buf[256] = {};
        // "common/name" triggers Steam's auto-localization path.
        const std::int64_t len = o_GetAppDataFromAppInfo(
            g_state.gameName.appInfoCacheObj.load(), appId, "common/name",
            reinterpret_cast<std::uint8_t*>(buf), static_cast<std::int32_t>(sizeof(buf)));
        // Return value is written size including trailing NUL when successful.
        if (len > 1) {
            name.assign(buf, static_cast<std::size_t>(len - 1));
        }
    }

    {
        std::lock_guard<std::mutex> lock(g_state.gameName.cacheMutex);
        g_state.gameName.nameCache[appId] = name;
    }
    if (!name.empty()) {
        AC_LOG_DEBUG_ONCE(kModule, "App %u name='%s'.", appId, name.c_str());
    }
    return name;
}

}  // namespace ac::gamename
