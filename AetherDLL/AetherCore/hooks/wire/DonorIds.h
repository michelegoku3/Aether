#pragma once

#include <array>
#include <cstdint>

// ---------------------------------------------------------------------------
// DonorIds — the SteamIDs AetherCore ships as the achievement donor pool.
//
// Data only, no logic: this is the one place to add a donor. The order is a
// contract, because per-app learning persists donor INDICES (donors.txt): a
// donor is appended at the end, never inserted or reordered, so an index keeps
// pointing at the same account across releases. Reordering or removing an entry
// changes the pool fingerprint, which discards the stale learning instead of
// spoofing the wrong donor.
//
// Slot 14 is LumaCore's default stat SteamID and the slot every fresh app starts
// from (see kDefaultPoolIndex in DonorPool.h). A donor added here is tried right
// after it, before the walk wraps back to slot 0.
// ---------------------------------------------------------------------------
namespace ac::hooks::DonorPool {

// The 15-entry LumaCore fallback pool, byte-identical to its
// kLumaCoreStatSteamIdPool, plus the donors discovered since: a game nobody in
// the original list owns gets its owner appended here.
inline constexpr std::array<std::uint64_t, 16> kCompiledDonors = {
    76561198017975643ULL,
    76561198001678750ULL,
    76561198355953202ULL,
    76561197979911851ULL,
    76561198040673812ULL,
    76561198367471798ULL,
    76561198028125071ULL,
    76561198012616627ULL,
    76561197971398453ULL,
    76561197977849691ULL,
    76561198019373005ULL,
    76561198155124847ULL,
    76561198063534772ULL,
    76561198072711049ULL,
    76561198028121353ULL,
    // The Loopler (4150860): none of the LumaCore donors owns it, this one does.
    76561197997477460ULL,
};

} // namespace ac::hooks::DonorPool
