#include "pch.h"
#include "hooks/ipc/CmdUser.h"

#include <chrono>
#include <cstring>
#include <iterator>
#include <mutex>
#include <span>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "credentials/CredentialStore.h"
#include "network/EticketFetcher.h"
#include "hooks/ipc/IPCBus.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/ipc/SteamCapture.h"
#include "credentials/SteamId.h"
#include "credentials/Ticket.h"

namespace ac::hooks::CmdUser {
namespace {

constexpr const char* kModule = "IPC.User";
using namespace ac::constants;

void LogGetSteamIdOnce(steam::AppId appId, std::uint64_t spoofed) {
    AC_LOG_DEBUG_ONCE(kModule, "GetSteamID: app %u -> %llu", appId,
                static_cast<unsigned long long>(spoofed));
}

// IClientUser::GetSteamID
//   Request:  none
//   Response: [tag][uint64 SteamID]  (9 bytes)
void GetSteamID(steam::CSteamPipeClient* pipe, steam::CUtlBuffer*, steam::CUtlBuffer* pWrite) {
    const steam::AppId appId = pipewatch::AppIdForPipe(pipe);
    const std::uint64_t spoofed = steamid::GetSpoofSteamId(appId);
    if (!spoofed) {
        AC_LOG_WARN_ONCE(kModule, "GetSteamID: no SteamID for app %u; leaving reply untouched.", appId);
        return;
    }
    if (!pWrite || !pWrite->Base() || pWrite->TellPut() < 9) return;

    std::uint8_t* base = pWrite->Base();
    base[0] = kIpcReplyTag;
    std::memcpy(base + 1, &spoofed, sizeof(spoofed));
    LogGetSteamIdOnce(appId, spoofed);
}

// IClientUser::GetAppOwnershipTicketExtendedData
//   Request args: [uint32 appId][int32 bufSize]
//   Response: [tag][uint32 ticketSize][ticket bytes ...][offsets block (16 b)]
void GetAppOwnershipTicketExtendedData(steam::CSteamPipeClient*, steam::CUtlBuffer* pRead,
                                       steam::CUtlBuffer* pWrite) {
    if (!pRead || !pRead->Base() || pRead->TellPut() < kIpcArgsOffset + 8) return;
    const std::uint8_t* args = pRead->Base() + kIpcArgsOffset;

    std::uint32_t reqAppId = 0;
    std::int32_t reqBufSize = 0;
    std::memcpy(&reqAppId, args, 4);
    std::memcpy(&reqBufSize, args + 4, 4);
    if (reqBufSize < 0 || static_cast<std::uint64_t>(reqBufSize) > ticket::kMaxAppTicketBytes) {
        AC_LOG_WARN_ONCE(kModule, "TicketExtendedData: invalid caller buffer size for app %u.", reqAppId);
        return;
    }
    if (!luadata::HasDepot(reqAppId)) {
        AC_LOG_WARN_ONCE(kModule, "TicketExtendedData: requested app %u is not configured; leaving reply untouched.",
                    reqAppId);
        return;
    }

    ticket::OwnershipTicket ownership{};
    if (!ticket::GetAppOwnershipTicket(reqAppId, ownership) || ownership.data.size() < 4) {
        AC_LOG_WARN_ONCE(kModule, "TicketExtendedData: app %u has no usable ticket.", reqAppId);
        return;
    }

    const std::vector<std::uint8_t>& ticket = ownership.data;
    const std::uint32_t ticketSize = static_cast<std::uint32_t>(ticket.size());

    // Reply must fit: tag + size + caller buffer + 16-byte offset block.
    const std::uint32_t total = 1 + 4 + static_cast<std::uint32_t>(reqBufSize) + 16;
    if (!pWrite || !pWrite->Base() || pWrite->TellPut() < 0 ||
        static_cast<std::uint32_t>(pWrite->TellPut()) < total) {
        AC_LOG_WARN_ONCE(kModule, "TicketExtendedData: reply buffer too small for app %u.", reqAppId);
        return;
    }

    const std::uint32_t returnSize = ownership.totalSize ? ownership.totalSize : ticketSize;

    std::uint8_t* base = pWrite->Base();
    base[0] = kIpcReplyTag;
    std::memcpy(base + 1, &returnSize, 4);

    const std::uint32_t copySize =
        ticketSize < static_cast<std::uint32_t>(reqBufSize) ? ticketSize
                                                            : static_cast<std::uint32_t>(reqBufSize);
    std::memcpy(base + 5, ticket.data(), copySize);
    if (copySize < static_cast<std::uint32_t>(reqBufSize)) {
        std::memset(base + 5 + copySize, 0, static_cast<std::uint32_t>(reqBufSize) - copySize);
    }

    // Offsets block Steam expects right after the ticket buffer.
    const std::uint32_t piAppId = ownership.appIdOffset;
    const std::uint32_t piSteamId = ownership.steamIdOffset;
    const std::uint32_t piSignature = ownership.signatureOffset;
    const std::uint32_t pcbSignature = ownership.signatureSize;
    std::uint8_t* tail = base + 5 + reqBufSize;
    std::memcpy(tail + 0, &piAppId, 4);
    std::memcpy(tail + 4, &piSteamId, 4);
    std::memcpy(tail + 8, &piSignature, 4);
    std::memcpy(tail + 12, &pcbSignature, 4);

    AC_LOG_DEBUG_ONCE(kModule, "TicketExtendedData: app %u -> %u bytes (return=%u, appOff=%u).",
                reqAppId, ticketSize, returnSize, piAppId);
}

// IClientUser::RequestEncryptedAppTicket
//   The reply buffer carries [tag][uint64 hAsyncCall]; we record the mapping so
//   the later GetAPICallResult(EncryptedAppTicketResponse) can answer OK.
void RequestEncryptedAppTicket(steam::CSteamPipeClient* pipe, steam::CUtlBuffer* pRead,
                               steam::CUtlBuffer* pWrite) {
    const steam::AppId appId = pipewatch::AppIdForPipe(pipe);
    if (!pWrite || !pWrite->Base() || pWrite->TellPut() < 9) return;

    // Request layout after the IPC header: [u32 nonceLen][nonce bytes...].
    // If a Lua-configured backend exists, try to mint a fresh ETicket/AppTicket
    // before falling back to whatever is already cached in the registry.
    if (pRead && pRead->Base() && pRead->TellPut() >= kIpcArgsOffset + 4) {
        const std::uint8_t* args = pRead->Base() + kIpcArgsOffset;
        std::uint32_t nonceLen = 0;
        std::memcpy(&nonceLen, args, sizeof(nonceLen));
        if (nonceLen > 0 && nonceLen <= 1024 &&
            pRead->TellPut() >= static_cast<std::int32_t>(kIpcArgsOffset + 4 + nonceLen)) {
            std::span<const std::uint8_t> nonce(args + 4, nonceLen);
            eticketfetch::Mint(appId, nonce);
        }
    }

    if (credential::ReadEncryptedTicket(appId).empty()) {
        AC_LOG_WARN_ONCE(kModule, "RequestEncryptedAppTicket: app %u has no eticket.", appId);
        return;
    }

    std::uint64_t asyncCall = 0;
    std::memcpy(&asyncCall, pWrite->Base() + 1, sizeof(asyncCall));
    if (!RememberETicketAsyncCall(asyncCall, appId)) {
        AC_LOG_WARN_ONCE(kModule, "RequestEncryptedAppTicket: rejected async=0x%llx app=%u.",
                         static_cast<unsigned long long>(asyncCall), appId);
        return;
    }
    AC_LOG_DEBUG_ONCE(kModule, "RequestEncryptedAppTicket: app %u async=0x%llx recorded.", appId,
                static_cast<unsigned long long>(asyncCall));
}

// IClientUser::GetEncryptedAppTicket
//   Response: [tag][1][uint32 size][ticket bytes ...]
void GetEncryptedAppTicket(steam::CSteamPipeClient* pipe, steam::CUtlBuffer*,
                           steam::CUtlBuffer* pWrite) {
    const steam::AppId appId = pipewatch::AppIdForPipe(pipe);
    std::vector<std::uint8_t> ticket = credential::ReadEncryptedTicket(appId);
    if (ticket.empty()) {
        AC_LOG_WARN_ONCE(kModule, "GetEncryptedAppTicket: app %u has no eticket.", appId);
        return;
    }

    const std::uint32_t ticketSize = static_cast<std::uint32_t>(ticket.size());
    const std::int32_t total = 1 + 1 + 4 + static_cast<std::int32_t>(ticketSize);
    capture::EnsureBufferSize(pWrite, total);
    if (!pWrite || !pWrite->Base() || pWrite->TellPut() < total) {
        AC_LOG_WARN_ONCE(kModule, "GetEncryptedAppTicket: response buffer unavailable for app %u.", appId);
        return;
    }

    std::uint8_t* base = pWrite->Base();
    base[0] = kIpcReplyTag;
    base[1] = 1;
    std::memcpy(base + 2, &ticketSize, sizeof(ticketSize));
    std::memcpy(base + 6, ticket.data(), ticketSize);
    AC_LOG_DEBUG_ONCE(kModule, "GetEncryptedAppTicket: app %u -> %u bytes.", appId, ticketSize);
}

const IpcHandlerEntry kEntries[] = {
    {ipc_iface::kClientUser, ipc_hash::kClientUser_GetSteamID,
     "IClientUser::GetSteamID", GetSteamID},
    {ipc_iface::kClientUser, ipc_hash::kClientUser_GetAppOwnershipTicketExtendedData,
     "IClientUser::GetAppOwnershipTicketExtendedData", GetAppOwnershipTicketExtendedData},
    {ipc_iface::kClientUser, ipc_hash::kClientUser_RequestEncryptedAppTicket,
     "IClientUser::RequestEncryptedAppTicket", RequestEncryptedAppTicket},
    {ipc_iface::kClientUser, ipc_hash::kClientUser_GetEncryptedAppTicket,
     "IClientUser::GetEncryptedAppTicket", GetEncryptedAppTicket},
};

}  // namespace

void Register() {
    RegisterIpcHandlers(kEntries, std::size(kEntries));
}

namespace {

constexpr std::size_t kMaxPendingETickets = 32;
constexpr auto kPendingETicketTtl = std::chrono::seconds(60);

void PruneExpiredLocked(const std::chrono::steady_clock::time_point now) {
    auto& state = g_state.pendingETickets;
    for (auto it = state.entries.begin(); it != state.entries.end();) {
        if (now - it->second.createdAt >= kPendingETicketTtl) {
            it = state.entries.erase(it);
            ++state.expiredCount;
        } else {
            ++it;
        }
    }
}

}  // namespace

bool RememberETicketAsyncCall(std::uint64_t asyncCall, steam::AppId appId) {
    if (asyncCall == 0 || appId == 0) {
        std::lock_guard<std::mutex> lock(g_state.pendingETickets.mutex);
        ++g_state.pendingETickets.rejectedCount;
        return false;
    }

    const auto now = std::chrono::steady_clock::now();
    std::lock_guard<std::mutex> lock(g_state.pendingETickets.mutex);
    auto& state = g_state.pendingETickets;
    PruneExpiredLocked(now);

    if (auto existing = state.entries.find(asyncCall); existing != state.entries.end()) {
        // A handle must identify one logical request. Replacing it is safer than
        // retaining stale app data, but is still observable in diagnostics.
        existing->second = {appId, now};
        ++state.recordedCount;
        return true;
    }

    if (state.entries.size() >= kMaxPendingETickets) {
        auto oldest = state.entries.begin();
        for (auto it = std::next(state.entries.begin()); it != state.entries.end(); ++it) {
            if (it->second.createdAt < oldest->second.createdAt) oldest = it;
        }
        state.entries.erase(oldest);
        ++state.evictedCount;
    }

    state.entries.emplace(asyncCall, AetherCoreState::PendingETicket{appId, now});
    ++state.recordedCount;
    return true;
}

std::optional<steam::AppId> ClaimETicketAsyncCall(std::uint64_t asyncCall) {
    if (asyncCall == 0) return std::nullopt;

    const auto now = std::chrono::steady_clock::now();
    std::lock_guard<std::mutex> lock(g_state.pendingETickets.mutex);
    auto& state = g_state.pendingETickets;
    PruneExpiredLocked(now);

    const auto it = state.entries.find(asyncCall);
    if (it == state.entries.end()) return std::nullopt;

    const steam::AppId appId = it->second.appId;
    state.entries.erase(it);
    ++state.claimedCount;
    return appId;
}

void ForgetETicketAsyncCall(std::uint64_t asyncCall) {
    if (asyncCall == 0) return;
    std::lock_guard<std::mutex> lock(g_state.pendingETickets.mutex);
    g_state.pendingETickets.entries.erase(asyncCall);
}

void ResetETicketAsyncCalls() {
    std::lock_guard<std::mutex> lock(g_state.pendingETickets.mutex);
    g_state.pendingETickets.entries.clear();
}

std::size_t PendingETicketAsyncCallCount() {
    const auto now = std::chrono::steady_clock::now();
    std::lock_guard<std::mutex> lock(g_state.pendingETickets.mutex);
    PruneExpiredLocked(now);
    return g_state.pendingETickets.entries.size();
}

}  // namespace ac::hooks::CmdUser
