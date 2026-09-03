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

// SteamUI deferred redirect retry (A7): max wall-clock budget for waiting on
// steamui.dll before installing the LoadModuleWithPath redirect in a second
// hook batch. The poll uses kSteamUiPollIntervalMs ticks.
inline constexpr int kSteamUiDeferredTimeoutMs = 30000;

// Package-0 startup retry (A2): re-attempt cadence and budget for the top-up
// when the first LoadPackage/MarkLicenseAsChanged window was missed (offline
// startup, late login, package 0 not ready yet). The retry thread wakes every
// 10 ms (granular stop) but only re-attempts at most once per interval.
inline constexpr int kPackageRetryIntervalMs = 1000;
inline constexpr int kPackageRetryMaxAttempts = 60;  // ~60 s budget

// Late pattern availability: cadence and budget for re-probing the pattern
// sources when a module table was missing at init (patterns not published
// yet on a fresh Steam build, or offline start). The moment a table appears,
// the previously-missed hooks are registered+installed in-session (no Steam
// restart) and the package-0 startup retry is re-armed.
inline constexpr int kPatternLateRetryIntervalMs = 10000;
inline constexpr int kPatternLateRetryMaxAttempts = 60;  // ~10 min budget

// Ownership unlock summary: debounce before emitting the per-file
// "Unlocked all / Not unlocked" summary after a burst of CheckAppOwnership
// calls (login, hot-reload, game launch). Keeps the log quiet while still
// settling late unlocks.
inline constexpr int kUnlockSummaryDebounceMs = 1500;
inline constexpr int kUnlockSummaryTickMs = 50;

// PipeWatch (A5): max snapshots before eviction kicks in. Steam itself holds
// ~5 pipes (steam.exe, steamwebhelper, gameoverlayui, …); each launched game
// adds 1–3 more (launcher, game exe, child overlay). 64 comfortably covers
// 20+ game launches in a single Steam session without unbounded growth.
inline constexpr std::size_t kPipeWatchMaxSnapshots = 64;

// Hasher: streaming chunk size for SHA-256 (4 MiB balances syscalls vs RAM).
inline constexpr std::size_t kHashChunkBytes = 4u * 1024u * 1024u;

// Pattern downloader: hard cap on a single TOML body to avoid unbounded reads.
inline constexpr std::size_t kMaxPatternResponseBytes = 1u * 1024u * 1024u;

// Lifecycle: how long DllMain(DETACH) waits for the init thread to unwind.
inline constexpr DWORD kInitThreadJoinTimeoutMs = 5000;

// Spacewar: Valve's public sample app id, used as the AetherOnline mask target.
inline constexpr std::uint32_t kSpacewarAppId = 480;

// AetherOnline (Aether's own masked-online mode, "-aetheronline"): the launch
// flag that opts a title into the 480-masking path. Named to stay distinct
// from the online-fix.me (OFME) crack.
inline constexpr char kAetherOnlineFlag[] = "-aetheronline";

// ShowOnline: launch flag that advertises the game to friends as "now
// playing" (Spacewar/480 on the SERVER-side presence only) WITHOUT any
// process masking — the client keeps the real appid everywhere else. Handled
// by the wire-level rewrite inside hooks/wire/GamesPlayedModule.
inline constexpr char kShowOnlineFlag[] = "-showonline";

// Suffix appended to game_extra_info for masked (480) sessions: it travels
// server-side inside Friend.game_name and lets Aether-equipped friends
// recover the exact real appid (no title guessing, no shared .lua).
// Format: "<display name> | <appid decimal>" — see GamesPlayedModule.
inline constexpr char kExtraInfoAppIdSep[] = " | ";

// Invisible suffix channel (docs/05-showonline-suffix-plan.md §9), the
// vanilla-clean alternative to the ASCII suffix above: the appid is appended
// as one U+200B ZERO WIDTH SPACE (UTF-8 E2 80 8B — also detaches the VS chain
// from the last visible base char, so FE0F can't restyle a trailing ®© or
// digit) plus exactly 6 Variation Selectors U+FE00..U+FE0F (UTF-8 EE B8 8n),
// one 24-bit nibble each, big-endian. Both are Default-Ignorable format
// characters: every compliant renderer (CEF friends UI included) draws
// NOTHING. Aether decodes the 6 nibbles back to the exact appid.
inline constexpr char kExtraInfoInvisibleMark[] = "\xE2\x80\x8B";  // U+200B
inline constexpr std::size_t kExtraInfoInvisibleDigits = 6;        // 24-bit / 4

// Preferred appid channel (docs/05-showonline-suffix-plan.md §10): hide the
// appid in CMsgClientGamesPlayed.GamePlayed.game_data_blob (field 8) — a
// raw-bytes slot the CM recycles into Friend.game_data_blob (field 60) for
// masked sessions exactly like it recycles game_extra_info into game_name.
// NO client UI ever renders it, so vanilla friends see just the plain name
// (no suffix, no appid — and no font-inventory pitfalls; the
// U+200B+VariationSelector encoding above fails there, it draws tofu).
// Format: magic "AETR", version byte (=1), appid little-endian (4 bytes).
inline constexpr char kAppIdBlobMagic[] = "AETR";
inline constexpr std::uint8_t kAppIdBlobVersion = 1;
inline constexpr std::size_t kAppIdBlobLen = 9;  // 4 + 1 + 4

// GameID layout: the low 24 bits of a Steam GameID carry the AppId.
inline constexpr std::uint64_t kGameIdAppIdMask = 0xFFFFFFull;

// Callback id Steam fires when app licenses change.
inline constexpr int kCallbackAppLicensesChanged = 1020094;

// Steamworks achievement/stats callback IDs. Used by the AetherOnline dual-dispatch
// in SendCallbackToPipe: when a game is masked as Spacewar/480, it registers
// callbacks under appid 480, but Steam dispatches them with the real appid.
// The dual-dispatch rewrites m_nGameID (low 24 bits) from real → 480 and
// re-emits so the game's 480-registered handlers also fire.
namespace achievement_cb {
inline constexpr int kUserStatsReceived      = 1101;
inline constexpr int kUserStatsStored        = 1102;
inline constexpr int kUserAchievementStored  = 1103;
inline constexpr int kUserAchievementIconFetched = 1109;

inline bool IsAchievementCallback(int cb) {
    return cb == kUserStatsReceived || cb == kUserStatsStored ||
           cb == kUserAchievementStored || cb == kUserAchievementIconFetched;
}
}  // namespace achievement_cb

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
// Steam's protocol constant for IClientUserStats (matches LumaCore's
// EIPCInterface). Used by IPCBus to bracket IClientUserStats dispatches in the
// AetherOnline stats-scope (see capture::EnterStatsScope) so the client's stats
// subsystem resolves the real app id instead of the Spacewar/480 mask.
inline constexpr std::uint8_t kClientUserStats = 11;
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
