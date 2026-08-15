#include "pch.h"
#include "credentials/SteamId.h"

#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include "credentials/CredentialStore.h"
#include "scripting/LuaData.h"
#include "core/Logger.h"
#include "credentials/Ticket.h"

namespace ac::steamid {
namespace {

constexpr const char* kModule = "SteamId";

void LogResolutionOnce(steam::AppId appId, const char* source, std::uint64_t value) {
    if (value != 0) {
        AC_LOG_DEBUG_ONCE(kModule, "App %u SteamID from %s -> %llu", appId, source,
                     static_cast<unsigned long long>(value));
    } else {
        AC_LOG_DEBUG_ONCE(kModule, "App %u: no SteamID resolvable.", appId);
    }
}

// Cached HKCU\...\Apps\<appId>\SteamID (REG_SZ). May be stale after account switch.
std::uint64_t FromAppSteamIdValue(steam::AppId appId) {
    return credential::ReadAppSteamIdValue(appId);
}

// SteamID baked into the cached AppOwnershipTicket at offset 8.
std::uint64_t FromOwnershipTicket(steam::AppId appId) {
    std::vector<std::uint8_t> ticket = credential::ReadAppOwnershipTicket(appId);
    if (ticket.size() < ticket::kAppTicketSteamIdOffset + sizeof(std::uint64_t))
        return 0;
    std::uint64_t id = 0;
    std::memcpy(&id, ticket.data() + ticket::kAppTicketSteamIdOffset, sizeof(id));
    return id;
}

// A userdata\<accountId>\<appId>\ folder means this account has played it.
std::uint64_t FromUserdataFolder(steam::AppId appId) {
    const std::string steamPath = credential::ReadSteamPath();
    if (steamPath.empty()) return 0;

    const std::string pattern = steamPath + "\\userdata\\*";
    WIN32_FIND_DATAA fd;
    HANDLE find = FindFirstFileA(pattern.c_str(), &fd);
    if (find == INVALID_HANDLE_VALUE) return 0;

    std::uint64_t result = 0;
    do {
        if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) || fd.cFileName[0] == '.')
            continue;
        char* end = nullptr;
        unsigned long accountId = std::strtoul(fd.cFileName, &end, 10);
        if (!end || *end != '\0' || accountId == 0) continue;

        const std::string gameDir =
            steamPath + "\\userdata\\" + fd.cFileName + "\\" + std::to_string(appId);
        if (GetFileAttributesA(gameDir.c_str()) != INVALID_FILE_ATTRIBUTES &&
            (GetFileAttributesA(gameDir.c_str()) & FILE_ATTRIBUTE_DIRECTORY)) {
            result = steam::MakeSteamId64(static_cast<std::uint32_t>(accountId));
            break;
        }
    } while (FindNextFileA(find, &fd));
    FindClose(find);
    return result;
}

// Keep Apps\<appId>\SteamID aligned with the live identity so later cold-start
// fallbacks are less stale. Best-effort; resolution does not depend on write.
void RefreshAppSteamIdCache(steam::AppId appId, std::uint64_t activeId) {
    if (activeId == 0) return;
    const std::uint64_t cached = FromAppSteamIdValue(appId);
    if (cached == activeId) return;
    const bool wrote = credential::WriteAppSteamIdValue(appId, activeId);
    AC_LOG_DEBUG_ONCE(
        kModule,
        "App %u: refreshed SteamID cache %llu -> %llu (wrote=%d).",
        appId,
        static_cast<unsigned long long>(cached),
        static_cast<unsigned long long>(activeId),
        wrote ? 1 : 0);
}

}  // namespace

std::uint64_t GetActiveSteamId64() {
    // 1. Live ActiveUser DWORD while Steam is running.
    if (std::uint32_t accountId = credential::ReadActiveUserId()) {
        return steam::MakeSteamId64(accountId);
    }

    // 2. Most recently modified userdata folder (Steam closed).
    const std::string steamPath = credential::ReadSteamPath();
    if (steamPath.empty()) return 0;

    const std::string pattern = steamPath + "\\userdata\\*";
    WIN32_FIND_DATAA fd;
    HANDLE find = FindFirstFileA(pattern.c_str(), &fd);
    if (find == INVALID_HANDLE_VALUE) return 0;

    DWORD bestAccount = 0;
    FILETIME bestTime{};
    do {
        if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) || fd.cFileName[0] == '.')
            continue;
        char* end = nullptr;
        unsigned long account = std::strtoul(fd.cFileName, &end, 10);
        if (!end || *end != '\0' || account == 0) continue;
        if (bestAccount == 0 || CompareFileTime(&fd.ftLastWriteTime, &bestTime) > 0) {
            bestAccount = static_cast<DWORD>(account);
            bestTime = fd.ftLastWriteTime;
        }
    } while (FindNextFileA(find, &fd));
    FindClose(find);

    return bestAccount ? steam::MakeSteamId64(bestAccount) : 0;
}

std::uint64_t GetSpoofSteamId(steam::AppId appId) {
    if (!luadata::HasDepot(appId)) return 0;

    // 1. Current account identity is authoritative. Cached per-app values can
    //    belong to a previous login on the same machine and must not win.
    if (std::uint64_t id = GetActiveSteamId64()) {
        RefreshAppSteamIdCache(appId, id);
        LogResolutionOnce(appId, "active", id);
        return id;
    }

    // 2–4. Offline / unresolved-active fallbacks only.
    if (std::uint64_t id = FromAppSteamIdValue(appId)) {
        LogResolutionOnce(appId, "registry", id);
        return id;
    }
    if (std::uint64_t id = FromOwnershipTicket(appId)) {
        LogResolutionOnce(appId, "ticket", id);
        return id;
    }
    if (std::uint64_t id = FromUserdataFolder(appId)) {
        LogResolutionOnce(appId, "userdata", id);
        return id;
    }

    LogResolutionOnce(appId, "none", 0);
    return 0;
}

}  // namespace ac::steamid
