#pragma once

#include <cstddef>
#include <cstdint>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Ticket inspection and forge — binary-level operations on AppOwnershipTicket
// blobs.  Registry I/O lives in CredentialStore so this module has zero
// registry code.  (Audit §3.2, 2026-07-12.)
//
// Steam caches per-app tickets under:
//   HKCU\Software\Valve\Steam\Apps\<appId>\AppTicket   (REG_BINARY)
//   HKCU\Software\Valve\Steam\Apps\<appId>\ETicket     (REG_BINARY)
// CredentialStore handles those reads/writes.
// ---------------------------------------------------------------------------
namespace ac::ticket {

// Offsets inside a standard AppOwnershipTicket blob.
inline constexpr std::uint32_t kAppTicketSteamIdOffset = 8;
inline constexpr std::uint32_t kAppTicketAppIdOffset = 16;
inline constexpr std::uint32_t kAppTicketSignatureSize = 128;
inline constexpr std::size_t kMaxAppTicketBytes = 1u * 1024u * 1024u;

enum class AppTicketStatus {
    Empty,
    TooSmall,
    InvalidLayout,
    SteamIdMismatch,
    AppIdMismatch,
    OkStandard,
    OkForged,
};

struct AppTicketInspection {
    AppTicketStatus status = AppTicketStatus::Empty;
    std::uint64_t steamId = 0;
    steam::AppId standardAppId = 0;
    steam::AppId forgedAppId = 0;
    std::uint32_t signatureOffset = 0;
    std::uint32_t forgedAppIdOffset = 0;
};

struct OwnershipTicket {
    std::vector<std::uint8_t> data;
    std::uint32_t totalSize = 0;
    std::uint32_t appIdOffset = kAppTicketAppIdOffset;
    std::uint32_t steamIdOffset = kAppTicketSteamIdOffset;
    std::uint32_t signatureOffset = 0;
    std::uint32_t signatureSize = kAppTicketSignatureSize;
};

// Validates a raw ticket blob against the expected appId and SteamID.
AppTicketInspection InspectAppOwnershipTicket(const std::vector<std::uint8_t>& data,
                                             steam::AppId appId,
                                             std::uint64_t expectedSteamId);

// Best-effort ownership ticket for appId: reads from CredentialStore, forges
// from a valid source (AppID 7 or any other owned app) when no direct ticket
// exists, and fills metadata offsets so the IPC reply can serve it correctly.
// Returns false when no usable ticket can be obtained.
bool GetAppOwnershipTicket(steam::AppId appId, OwnershipTicket& ticket);

// Forges a ticket for targetAppId by inserting the target app id just before
// the signature of a valid sourceAppId ticket. Returns an empty vector on
// failure (source missing, incompatible, or SteamID mismatch).
std::vector<std::uint8_t> ForgeAppOwnershipTicket(steam::AppId sourceAppId,
                                                  steam::AppId targetAppId);

}  // namespace ac::ticket
