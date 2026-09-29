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
// Out-param of CClientUser::CheckAppOwnership.
//
// FIELD ORDER IS EVIDENCE, NOT PREFERENCE. Offsets and the extent of the bool
// block are proven by disassembly of both shipped builds (rounds 3-6); the
// NAMES of several fields are deductions from the branch that writes them,
// and the ones that are not certain say so. Getting a name wrong here does
// not crash: it makes Steam believe something false. That already happened —
// `borrowed` was declared at +0x31, which is a byte of unknown meaning, and
// Steam started showing the family-sharing banner on .lua apps.
//
// Aether READS:  releaseState, existInPackageNums, borrowed, familyShared
// Aether WRITES: packageId, releaseState, freeLicense, ownsLicense
// Everything else is layout: declared so the offsets are right, never touched.
struct AppOwnership {
    // ---- scalars (offsets proven; names for +0x0C/+0x14 are not) ----------
    std::uint32_t packageId;                 // +0x00 proven (echoes the appId)
    AppReleaseState releaseState;            // +0x04 likely (enum from a KV string)
    std::uint32_t steamId32;                 // +0x08 UNKNOWN name (round 3: masterSubscriptionAppId?)
    std::uint32_t unknown_0x0C;              // +0x0C UNKNOWN
    std::uint32_t trialSeconds;              // +0x10 likely (DevComp / TimedTrialMinutes * 60)
    std::uint32_t existInPackageNums;        // +0x14 likely (a counter: incremented per entry)
    char purchaseCountryCode[4];             // +0x18 likely (word at +0x18, byte at +0x1A)
    std::uint32_t timeStamp;                 // +0x1C proven offset
    std::uint32_t timeExpire;                // +0x20 proven offset

    // ---- the eighteen booleans, +0x24..+0x35, contiguous (proven) --------
    // Valve only ever stores 0 or 1 here; StructGuard uses that to detect a
    // shifted layout.
    bool ownsLicense;                        // +0x24 likely  (= IsOwnedNow(license))
    bool licenseExpired;                     // +0x25 likely  (bit 2 of license flags)
    bool isPermanent;                        // +0x26 likely  (permanent/commercial helper)
    bool lowViolence;                        // +0x27 likely  (bit 6)
    bool freeLicense;                        // +0x28 guess   (defaults to 1, ANDed per package)
    bool licensePending;                     // +0x29 likely  (flags 0x100/0x200 path)
    bool fromFreeWeekend;                    // +0x2A likely  ("FreeWeekend" KV hit)
    bool licenseLocked;                      // +0x2B guess   (bit 0)
    bool regionRestricted;                   // +0x2C likely  (flags & 0x30)
    bool autoGrant;                          // +0x2D guess   (license type == 1)
    bool retailLicense;                      // +0x2E likely  (license type == 0x40)
    bool borrowed;                           // +0x2F likely  (set on the site/borrower path,
                                             //                round 7: setter 0x4C3000) <- read by Aether
    bool allActivationRequired;              // +0x30 guess   (defaults 1, ANDed with bit 11)
    bool timedLicense;                       // +0x31 likely  (license type 7: timed, ~30.7d grace)
    bool anySiteLicense;                     // +0x32 guess   (|= type == 0x50)
    bool allSiteLicenses;                    // +0x33 guess   (defaults 1, ANDed with type == 0x50)
    bool guestPass;                          // +0x34 guess   (defaults 1, ANDed with bit 12)
    bool familyShared;                       // +0x35 likely  (flags & 0x4000 + subscription
                                             //                match, round 7) <- read by Aether
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
//: The value PackageInfo::status holds when the "status" key was absent from
//: the package's KeyValues — i.e. the package is not loaded. Valve gates on
//: exactly this, never on a positive "available" constant.
inline constexpr std::uint32_t kPackageStatusUnloaded = 3;

struct PackageInfo {
    std::uint32_t packageId;
    std::int32_t changeNumber;
    std::uint64_t picsToken;
    std::uint32_t billingType;
    std::uint32_t licenseType;
    // Usability gate. There is NO "Available" constant: Valve's own code tests
    // `status != 3`, where 3 is what the "status" key lookup returns when the
    // key is absent (round-6 disassembly: LoadPackage writes it with default 3,
    // and every consumer — CheckAppOwnership x2, ProcessPendingLicenseUpdates,
    // 0x9E0B00 — discards the package on `== 3`). Offset +0x18, both builds.
    std::uint32_t status;
    std::uint8_t sha1[20];
    void* packageInfoNodeBegin;
    void* extendNodeBegin;
    CUtlVector<AppId> appIdVec;
    CUtlVector<AppId> depotIdVec;
};

// Depot entry. ManifestGid (offset 0x08) is the field we override.
// One row of the depot dependency table Steam builds before a download.
//
// Stride 0x20 is proven on both builds (the loops in BuildDepotDependency
// index with `shl reg,5`), and manifestGid/manifestSize at +0x08/+0x10 come
// from the filler helper that writes them (round 5). Aether OVERWRITES
// manifestGid — the id of the exact content snapshot Steam will download — so
// this struct gets the same treatment as the others: guarded, never trusted.
struct DepotEntry {
    std::uint32_t depotId;       // 0x00 proven (round 7, from the producer)
    std::uint32_t appId;         // 0x04 proven (round 7, from the producer)
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

// Incoming network packet handed to RecvPkt.
//
// DELIBERATELY OPAQUE — do not declare its fields here again.
//
// Steam beta 1790380355 inserted two uint32 after m_hConnection and shifted
// m_pubData/m_cubData/m_cRef by +8. The old flat definition (data @ +8,
// dataLen @ +16) was WRITTEN through by the wire hooks, so on that build it
// stored a pointer over a version stamp and dereferenced an integer as a
// buffer: Steam died before finishing boot.
//
// The two fields we use are reached through ac::abi::netpkt::Data()/Size(),
// which apply an offset identified at runtime from a live packet (and never
// fall back to a compiled default). See core/NetPacketAbi.h.
struct CNetPacket;

}  // namespace ac::steam
