#pragma once

#include <cstddef>
#include <cstdint>

// ---------------------------------------------------------------------------
// Minimal reverse-engineered Steam ABI types.
//
// Only the layouts AetherCore actually touches live here. Everything is plain
// data with documented offsets so that, when a Steam update shifts a struct,
// the fix is a localised edit instead of a hunt through hook bodies.
// ---------------------------------------------------------------------------
namespace ac::steam {

using AppId = std::uint32_t;
using PackageId = std::uint32_t;
using HSteamPipe = std::int32_t;
using HSteamUser = std::int32_t;

// High 32 bits of a SteamID64 for an individual account in the public universe
// (universe=1, type=1 individual, instance=1 desktop). The low 32 bits are the
// account id. Used to turn a bare account id into a full SteamID64.
inline constexpr std::uint64_t kSteamId64IndividualBase = 0x0110000100000000ull;

// Combines an account id with the individual/public/desktop prefix.
inline constexpr std::uint64_t MakeSteamId64(std::uint32_t accountId) {
    return kSteamId64IndividualBase | static_cast<std::uint64_t>(accountId);
}

// App release state as reported by CheckAppOwnership.
enum class AppReleaseState : std::uint32_t {
    Invalid = 0,
    Uninstalled = 1,
    // Steam reports the same value for "fully installed" and "released".
    Released = 4,
};

// Ownership record returned by CClientUser::CheckAppOwnership. Field order is
// dictated by Steam; do not reorder.
struct AppOwnership {
    std::uint32_t packageId;
    AppReleaseState releaseState;
    std::uint32_t steamId32;
    std::uint32_t masterSubscriptionAppId;
    std::uint32_t trialSeconds;
    std::uint32_t existInPackageNums;
    char purchaseCountryCode[4];
    std::uint32_t timeStamp;
    std::uint32_t timeExpire;
    bool ownsLicense;
    bool licenseExpired;
    bool isPermanent;
    bool lowViolence;
    bool freeLicense;
    bool regionRestricted;
    bool fromFreeWeekend;
    bool licenseLocked;
    bool licensePending;
    bool retailLicense;
    bool autoGrant;
    bool licensePermanent;
    bool guestPass;
    bool borrowed;
    bool anySiteLicense;
    bool allSiteLicenses;
    bool allActivationRequired;
    bool familyShared;
};

// Valve's CUtlMemory<T>: a growable, relocatable backing buffer.
template <class T>
struct CUtlMemory {
    T* memory;
    std::uint32_t allocationCount;
    std::uint32_t growSize;
};

// Valve's CUtlVector<T>: CUtlMemory plus an element count.
template <class T>
struct CUtlVector {
    CUtlMemory<T> mem;
    std::uint32_t size;

    // Swap-removes the first element equal to value (order not preserved, which
    // matches Valve's FastRemove). Returns true if an element was removed.
    bool FindAndFastRemove(const T& value) {
        for (std::uint32_t i = 0; i < size; ++i) {
            if (mem.memory[i] == value) {
                if (i != size - 1) mem.memory[i] = mem.memory[size - 1];
                --size;
                return true;
            }
        }
        return false;
    }
};

// Package metadata. AppIdVec is where we inject owned titles into package 0.
struct PackageInfo {
    std::uint32_t packageId;
    std::int32_t changeNumber;
    std::uint64_t picsToken;
    std::uint32_t billingType;
    std::uint32_t licenseType;
    std::uint32_t status;  // 0 == Available; only inject when available.
    std::uint8_t sha1[20];
    void* packageInfoNodeBegin;
    void* extendNodeBegin;
    CUtlVector<AppId> appIdVec;
    CUtlVector<AppId> depotIdVec;
};

// Depot entry. ManifestGid (offset 0x08) is the field we override.
struct DepotEntry {
    std::uint32_t depotId;       // 0x00
    std::uint32_t appId;         // 0x04
    std::uint64_t manifestGid;   // 0x08
    std::uint64_t manifestSize;  // 0x10
    std::uint32_t dlcAppId;      // 0x18
    std::uint8_t lcsRequired;    // 0x1C
    std::uint8_t notNewTarget;   // 0x1D
    std::uint8_t sharedInstall;  // 0x1E
    std::uint8_t padding;        // 0x1F
};

// Valve's CUtlBuffer used by the IPC layer to carry request/response payloads.
// We only need the memory base and the put cursor; the trailing members keep
// the layout correct so member offsets match Steam's struct.
struct CUtlBuffer {
    CUtlMemory<std::uint8_t> memory;
    std::int32_t get;
    std::int32_t put;     // bytes written; doubles as response capacity hint
    std::int32_t offset;
    std::int32_t flags;
    void* getOverflowFunc;
    void* putOverflowFunc;

    std::uint8_t* Base() { return memory.memory; }
    const std::uint8_t* Base() const { return memory.memory; }
    std::int32_t TellPut() const { return put; }
};

// Per-client IPC pipe descriptor. We only read m_hSteamPipe (offset 16); the
// padding preserves the offset across the rest of Steam's struct.
struct CSteamPipeClient {
    void* server;                  // +0
    void* client;                  // +8
    std::uint32_t hSteamPipe;      // +16
    std::uint8_t pad0[12];         // +20
    std::uint32_t clientPid;       // +32
    std::uint8_t pad1[4];          // +36
    char* processName;             // +40
};
static_assert(offsetof(CSteamPipeClient, hSteamPipe) == 16,
              "CSteamPipeClient::hSteamPipe must sit at offset 16");

// ---- Wire protocol (PacketRouter) -----------------------------------------

// WebSocket opcode passed to BBuildAndAsyncSendFrame; only binary frames carry
// Steam protobuf messages.
enum EWebSocketOpCode : char {
    k_eWebSocketOpCode_Binary = 0x02,
};

// Frame header. The high bit of eMsg flags a protobuf message; the low bits are
// the real EMsg. headerLength counts the CMsgProtoBufHeader that follows.
struct MsgHdr {
    std::uint32_t eMsg;
    std::uint32_t headerLength;
};
inline constexpr std::uint32_t kMsgHdrProtoFlag = 0x80000000u;

// Incoming network packet handed to RecvPkt. We only touch the data pointer and
// length; later members preserve layout.
struct CNetPacket {
    void* connection;          // +0
    std::uint8_t* data;        // +8
    std::uint32_t dataLen;     // +16
    std::int32_t refCount;     // +20
    std::uint8_t* networkBuffer;
    CNetPacket* next;
};

}  // namespace ac::steam
