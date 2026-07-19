#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// CredentialStore — centralised registry I/O for everything under
// HKCU\Software\Valve\Steam.
//
// Created during the 2026-07-12 audit refactor (§3.2). Before this module
// existed, Ticket.cpp and SteamId.cpp each had their own registry helpers
// (WriteBinary/ReadBinary in Ticket, ReadRegString in SteamId).  Now all
// reads and writes go through one place, and the two consumers focus on
// their actual job: Ticket inspects/forges binary ticket blobs, SteamId
// resolves SteamID64 identities from local sources.
//
// Thread-safe: every function that touches g_state locks the appropriate
// mutex (configStoreTicketMutex for the config-store cache).
// ---------------------------------------------------------------------------
namespace ac::credential {

// ---- Ticket blobs (REG_BINARY under Apps\<appId>) ---------------------------

bool WriteAppOwnershipTicket(steam::AppId appId, const std::vector<std::uint8_t>& data);
bool WriteEncryptedTicket(steam::AppId appId, const std::vector<std::uint8_t>& data);

// ReadAppOwnershipTicket checks the config-store cache first, then falls back
// to the registry.  ReadEncryptedTicket goes straight to the registry.
std::vector<std::uint8_t> ReadAppOwnershipTicket(steam::AppId appId);
std::vector<std::uint8_t> ReadEncryptedTicket(steam::AppId appId);

// ---- Config-store passive cache --------------------------------------------

// Passively caches a ticket Steam read from its user-local config store.
// Invalid / mismatched tickets are silently rejected.
bool CacheConfigStoreAppOwnershipTicket(steam::AppId appId,
                                        const std::vector<std::uint8_t>& data);

// Diagnostics for status.json.
std::size_t CachedConfigStoreTicketCount();

// ---- Steam identity --------------------------------------------------------

// Reads ActiveProcess\ActiveUser (REG_DWORD). Returns 0 on failure.
std::uint32_t ReadActiveUserId();

// Reads Apps\<appId>\SteamID (REG_SZ), parsed as decimal. Returns 0 if absent.
std::uint64_t ReadAppSteamIdValue(steam::AppId appId);

// Writes Apps\<appId>\SteamID (REG_SZ, decimal). Returns true on success.
// Refuses to write 0 (would poison the cache). Symmetric with
// ReadAppSteamIdValue; used by the per-app owner fallback to persist the
// active-user resolution so subsequent calls can short-circuit on the
// registry read instead of re-walking userdata.
bool WriteAppSteamIdValue(steam::AppId appId, std::uint64_t steamId);

// Reads the Steam installation path from the registry. Returns "" on failure.
std::string ReadSteamPath();

}  // namespace ac::credential
