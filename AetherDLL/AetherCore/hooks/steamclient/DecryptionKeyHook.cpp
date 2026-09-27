#include "pch.h"
#include "hooks/steamclient/DecryptionKeyHook.h"

#include <cstring>
#include <vector>

#include "core/AetherCoreState.h"
#include "credentials/CredentialStore.h"
#include "credentials/HexCodec.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "utils/PatternEngine.h"
#include "utils/SteamKeyPaths.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "ConfigStore";
constexpr std::int32_t kUserLocalConfigStore = 3;

using ConfigStoreGetBinary_t = std::int32_t (*)(void*, std::int32_t, const char*, char*, std::uint32_t);
ConfigStoreGetBinary_t o_ConfigStoreGetBinary = nullptr;

void CaptureStoreObj(void* self, std::int32_t storeType) {
    if (!self || storeType != kUserLocalConfigStore) return;
    void* prev = nullptr;
    if (g_state.pConfigStoreUserLocal.compare_exchange_strong(prev, self)) {
        AC_LOG_INFO(kModule, "Captured user-local ConfigStore object 0x%p.", self);
    }
}

std::int32_t h_ConfigStoreGetBinary(void* self, std::int32_t storeType,
                                    const char* keyName, char* keyOut, std::uint32_t keySize) {
    CaptureStoreObj(self, storeType);

    // First path: feed depot decryption keys directly when Steam asks through
    // ConfigStore instead of LoadDepotDecryptionKey.
    if (keyName && keyOut && keySize > 0) {
        if (auto depot = keypath::DepotIdFromDecryptionKeyName(keyName)) {
            if (auto hexKey = luadata::DepotKeyHex(*depot)) {
                if (auto bytes = ac::hex::Decode(*hexKey); bytes && bytes->size() <= keySize) {
                    std::memcpy(keyOut, bytes->data(), bytes->size());
                    AC_LOG_INFO(kModule, "Fed ConfigStore decryption key for depot %u (%zu bytes).",
                                *depot, bytes->size());
                    return static_cast<std::int32_t>(bytes->size());
                }
            }
        }
    }

    const std::int32_t got = o_ConfigStoreGetBinary(self, storeType, keyName, keyOut, keySize);

    // Second path: passively cache AppTickets that Steam reads from its
    // user-local config store.
    if (got > 0 && keyOut && keyName && storeType == kUserLocalConfigStore) {
        if (auto appId = keypath::AppIdFromAppTicketKeyName(keyName)) {
            std::vector<std::uint8_t> blob(static_cast<std::size_t>(got));
            std::memcpy(blob.data(), keyOut, static_cast<std::size_t>(got));
            credential::CacheConfigStoreAppOwnershipTicket(*appId, blob);
        }
    }

    return got;
}

}  // namespace

void RegisterDecryptionKeyHook(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering ConfigStoreGetBinary hook.");

    void* cfg = pattern::ResolveAddress("ConfigStoreGetBinary", "steamclient", diversion);
    if (!cfg) {
        g_state.hookManager.RecordMissed("ConfigStoreGetBinary", MissReason::PatternUnresolved);
        return;
    }

    if (void* depot = pattern::ResolveAddress("LoadDepotDecryptionKey", "steamclient", diversion)) {
        if (cfg == depot) {
            // Not a defect and not a pattern miss: disassembly of both builds
            // (stable 0x5C5F50 / beta 0x5D9010) shows Valve compiles
            // ConfigStoreGetBinary and LoadDepotDecryptionKey into ONE
            // function, so the LoadDepotDecryptionKey hook already covers this
            // code. Hooking here would hook the same function twice.
            // Reported as a known alias so it stays visible — the day the two
            // names split again, the entry disappears and we notice — without
            // inflating the missed-hook count.
            g_state.hookManager.RecordMissed("ConfigStoreGetBinary", MissReason::KnownAlias,
                                             "LoadDepotDecryptionKey");
            return;
        }
    }

    g_state.hookManager.RegisterHook("ConfigStoreGetBinary", cfg,
                               reinterpret_cast<void**>(&o_ConfigStoreGetBinary),
                               reinterpret_cast<void*>(h_ConfigStoreGetBinary));
}

}  // namespace ac::hooks
