#pragma once

#include <cstdint>

// ---------------------------------------------------------------------------
// Centralised compile-time constants.
//
// LumaCore scattered magic numbers across the whole tree (see DOCS_TODO
// 13-lumacore-technical-debt.md #10). AetherCore keeps every tunable in one
// place so a Steam update only ever requires editing this file, never hunting
// through hook bodies.
// ---------------------------------------------------------------------------
namespace ac::constants {

// Diversion: how aggressively we retry the steamclient copy/load. Steam may
// still hold a write lock on the freshly extracted DLL for a short window.
inline constexpr int kDiversionMaxRetries = 30;
inline constexpr int kDiversionRetryDelayMs = 100;

// SteamUI poll loop: interval used while waiting for steamui.dll to appear.
inline constexpr int kSteamUiPollIntervalMs = 100;

// Package-0 startup retry (A2): re-attempt cadence and budget for the top-up
// when the first LoadPackage/MarkLicenseAsChanged window was missed (offline
// startup, late login, package 0 not ready yet). The retry thread wakes every
// 10 ms (granular stop) but only re-attempts at most once per interval.
inline constexpr int kPackageRetryIntervalMs = 1000;
inline constexpr int kPackageRetryMaxAttempts = 60;  // ~60 s budget

// Ownership unlock summary: debounce before emitting the per-file
// "Unlocked all / Not unlocked" summary after a burst of CheckAppOwnership
// calls (login, hot-reload, game launch). Keeps the log quiet while still
// settling late unlocks.
inline constexpr int kUnlockSummaryDebounceMs = 1500;
inline constexpr int kUnlockSummaryTickMs = 50;

// Hasher: streaming chunk size for SHA-256 (4 MiB balances syscalls vs RAM).
inline constexpr std::size_t kHashChunkBytes = 4u * 1024u * 1024u;

// Pattern downloader: hard cap on a single TOML body to avoid unbounded reads.
inline constexpr std::size_t kMaxPatternResponseBytes = 1u * 1024u * 1024u;

// Lifecycle: how long DllMain(DETACH) waits for the init thread to unwind.
inline constexpr DWORD kInitThreadJoinTimeoutMs = 5000;

// Spacewar: Valve's public sample app id, used as the OnlineFix mask target.
inline constexpr std::uint32_t kSpacewarAppId = 480;

// OnlineFix: the launch flag that opts a title into the 480-masking path.
inline constexpr char kOnlineFixFlag[] = "-onlinefix";

// GameID layout: the low 24 bits of a Steam GameID carry the AppId.
inline constexpr std::uint64_t kGameIdAppIdMask = 0xFFFFFFull;

// Callback id Steam fires when app licenses change.
inline constexpr int kCallbackAppLicensesChanged = 1020094;

// ---------------------------------------------------------------------------
// IPC bus wire format (game <-> Steam).
//
// InterfaceCall packet header layout:
//   off 0:  cmd          (1 byte, see IpcCommand)
//   off 1:  interfaceId  (1 byte, see IpcInterface)
//   off 2:  hSteamUser   (4 bytes)
//   off 6:  funcHash     (4 bytes)
//   off 10: args[]       (variable)
// Replies begin with a single tag byte.
// ---------------------------------------------------------------------------
inline constexpr int kIpcOffsetCmd = 0;
inline constexpr int kIpcOffsetInterfaceId = 1;
inline constexpr int kIpcOffsetFuncHash = 6;
inline constexpr int kIpcArgsOffset = 10;
inline constexpr int kIpcHeaderSize = 10;
inline constexpr int kIpcHandshakePidOffset = 5;
inline constexpr int kIpcHandshakeMinSize = 9;
inline constexpr std::uint8_t kIpcReplyTag = 0x0B;

// Pipes whose low 16 bits are <= this value are Steam-internal traffic and must
// pass straight through without our handlers.
inline constexpr std::uint32_t kIpcInternalPipeMax = 2;

// IPC command ids (EIPCCommand).
namespace ipc_cmd {
inline constexpr std::uint8_t kInterfaceCall = 1;
inline constexpr std::uint8_t kHandshake = 9;
}  // namespace ipc_cmd

// IPC interface ids (EIPCInterface).
namespace ipc_iface {
inline constexpr std::uint8_t kClientUser = 1;
inline constexpr std::uint8_t kClientUtils = 4;
}  // namespace ipc_iface

// Pre-computed method-name hashes Steam uses for InterfaceCall dispatch.
namespace ipc_hash {
inline constexpr std::uint32_t kClientUser_GetSteamID = 0xD6FC3200;
inline constexpr std::uint32_t kClientUser_GetAppOwnershipTicketExtendedData = 0xC7E71245;
inline constexpr std::uint32_t kClientUser_RequestEncryptedAppTicket = 0x25D6BB1D;
inline constexpr std::uint32_t kClientUser_GetEncryptedAppTicket = 0xE0468CB4;
inline constexpr std::uint32_t kClientUtils_GetAppID = 0x09607EC4;
inline constexpr std::uint32_t kClientUtils_GetAPICallResult = 0x2D3D3947;
}  // namespace ipc_hash

// Callback ids relevant to the (non-achievement) IPC reply path.
inline constexpr std::uint32_t kCallbackEncryptedAppTicketResponse = 154;  // 100 + 54

// EResult::k_EResultOK.
inline constexpr std::uint32_t kEResultOk = 1;

// ---------------------------------------------------------------------------
// Wire protocol (PacketRouter) message ids and helpers.
// ---------------------------------------------------------------------------
namespace emsg {
inline constexpr std::uint32_t kMulti = 1;
inline constexpr std::uint32_t kServiceMethodResponse = 147;       // recv service jobs
inline constexpr std::uint32_t kServiceMethodCallFromClient = 151; // send service jobs
inline constexpr std::uint32_t kClientGetUserStats = 818;
inline constexpr std::uint32_t kClientGetUserStatsResponse = 819;
inline constexpr std::uint32_t kClientGamesPlayed = 742;           // presence stack
inline constexpr std::uint32_t kClientPersonaState = 766;          // rich presence
inline constexpr std::uint32_t kClientGamesPlayedWithDataBlob = 5410;
inline constexpr std::uint32_t kClientStoreUserStats2 = 5466;
inline constexpr std::uint32_t kClientRequestEncryptedAppTicketResponse = 5527; // eticket fallback
inline constexpr std::uint32_t kClientRichPresenceUpload = 7501;
inline constexpr std::uint32_t kClientPICSProductInfoRequest = 8903;  // access token
inline constexpr std::uint32_t kClientSharedLibraryLockStatus = 9405; // family sharing
inline constexpr std::uint32_t kClientSharedLibraryStopPlaying = 9406;
}  // namespace emsg

// EClientPersonaStateFlag::k_EClientPersonaStateFlagRichPresence — when set on
// an inbound PersonaState, the UI rebuilds m_mapRichPresence from the message's
// rich_presence() list. Clearing the bit on an inject with empty KVs avoids
// wiping an already-populated map (OpenSteamTool / LumaCore policy).
inline constexpr std::uint32_t kStatusFlagRichPresence = 0x1000;

// Compile-time FNV-1a (32-bit) used to dispatch service jobs by target_job_name.
inline constexpr std::uint32_t FnvHash(const char* s) {
    std::uint32_t h = 0x811c9dc5u;
    while (*s) {
        h ^= static_cast<std::uint32_t>(static_cast<unsigned char>(*s++));
        h *= 0x01000193u;
    }
    return h;
}

namespace job_hash {
inline constexpr std::uint32_t kNotifyRunningApps =
    FnvHash("FamilyGroupsClient.NotifyRunningApps#1");
inline constexpr std::uint32_t kGetManifestRequestCode =
    FnvHash("ContentServerDirectory.GetManifestRequestCode#1");
inline constexpr std::uint32_t kGetUserStats =
    FnvHash("Player.GetUserStats#1");
}  // namespace job_hash

// PacketRouter ring-buffer pool sizing. 256 KiB body covers large messages
// (e.g. big persona-state batches); 1 KiB header is ample for CMsgProtoBufHeader.
inline constexpr std::uint32_t kWireMaxBodyBytes = 262144;
inline constexpr std::uint32_t kWireMaxHeaderBytes = 1024;
inline constexpr int kWirePoolSlots = 8;


}  // namespace ac::constants
