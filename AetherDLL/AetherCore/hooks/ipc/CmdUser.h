#pragma once

#include <cstddef>
#include <cstdint>
#include <optional>

#include "core/SteamTypes.h"

// IClientUser IPC handlers: SteamID spoofing and ownership/encrypted ticket
// replies. Tickets and SteamIDs come from the registry-backed Ticket/SteamId
// modules, never fabricated.
namespace ac::hooks::CmdUser {

// Registers this module's handlers with the IPC bus.
void Register();

// ETicket async-call bookkeeping, consumed by CmdUtils GetAPICallResult.
// The implementation keeps this state bounded and expires abandoned calls.
bool RememberETicketAsyncCall(std::uint64_t asyncCall, steam::AppId appId);
std::optional<steam::AppId> ClaimETicketAsyncCall(std::uint64_t asyncCall);
void ForgetETicketAsyncCall(std::uint64_t asyncCall);
void ResetETicketAsyncCalls();
std::size_t PendingETicketAsyncCallCount();

}  // namespace ac::hooks::CmdUser
