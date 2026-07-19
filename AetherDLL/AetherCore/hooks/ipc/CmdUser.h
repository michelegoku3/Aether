#pragma once

#include <cstdint>

#include "core/SteamTypes.h"

// IClientUser IPC handlers: SteamID spoofing and ownership/encrypted ticket
// replies. Tickets and SteamIDs come from the registry-backed Ticket/SteamId
// modules, never fabricated.
namespace ac::hooks::CmdUser {

// Registers this module's handlers with the IPC bus.
void Register();

// eticket async-call bookkeeping, consumed by CmdUtils GetAPICallResult.
steam::AppId LookupETicketAsyncCall(std::uint64_t asyncCall);
void EraseETicketAsyncCall(std::uint64_t asyncCall);

}  // namespace ac::hooks::CmdUser
