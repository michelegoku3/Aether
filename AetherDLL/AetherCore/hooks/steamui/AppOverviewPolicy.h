#pragma once

#include <cstdint>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// SteamUI app-overview policy: the decisions, with no logger, no shared state
// and no Windows calls, so the unit suite exercises the real thing.
//
// The overview object is written only after InspectOverview() accepts it and
// the probe has latched on two distinct apps. A rejected sample is a layout
// move, never a value to preserve: the caller skips the write and reports it.
// ---------------------------------------------------------------------------
namespace ac::hooks::steamui::policy {

enum class OverviewVerdict {
    Ok,
    NullObject,
    AppIdZero,
    GameIdMismatch,
    OwnershipOutOfMask,
    StateOutOfMask,
};

inline const char* OverviewVerdictText(OverviewVerdict verdict) noexcept {
    switch (verdict) {
    case OverviewVerdict::Ok:
        return "ok";
    case OverviewVerdict::NullObject:
        return "null object";
    case OverviewVerdict::AppIdZero:
        return "appId is zero";
    case OverviewVerdict::GameIdMismatch:
        return "gameId and appId disagree (layout moved?)";
    case OverviewVerdict::OwnershipOutOfMask:
        return "ownership flags outside the known mask (layout moved?)";
    case OverviewVerdict::StateOutOfMask:
        return "app state outside the known mask (layout moved?)";
    }
    return "unknown";
}

// The appId is the low 24 bits of the 64-bit GameID; Steam never mints a
// larger app id, so a mismatch means we are not looking at the object we
// compiled against.
inline OverviewVerdict InspectOverview(const steam::AppOverviewPrefix* app) noexcept {
    if (!app)
        return OverviewVerdict::NullObject;
    if (app->appId == 0)
        return OverviewVerdict::AppIdZero;
    if ((app->gameId & 0x00FFFFFFull) != app->appId)
        return OverviewVerdict::GameIdMismatch;
    if ((static_cast<std::uint32_t>(app->ownershipFlags) & ~steam::kOwnershipKnownMask) != 0)
        return OverviewVerdict::OwnershipOutOfMask;
    if ((app->appStateFlags & ~steam::kAppStateKnownMask) != 0)
        return OverviewVerdict::StateOutOfMask;
    return OverviewVerdict::Ok;
}

// Evidence that these offsets are the live object: one matching app could be a
// coincidence, two different apps matching the same layout cannot.
struct ProbeState {
    std::uint32_t samples = 0;
    steam::AppId firstAppId = 0;
    bool latched = false;
};

inline bool RecordProbeSample(ProbeState& state, const steam::AppOverviewPrefix& app) noexcept {
    if (state.latched)
        return false;
    if (InspectOverview(&app) != OverviewVerdict::Ok) {
        state.samples = 0;
        state.firstAppId = 0;
        return false;
    }
    if (state.firstAppId == 0) {
        state.firstAppId = app.appId;
        state.samples = 1;
        return false;
    }
    if (app.appId != state.firstAppId) {
        state.latched = true;
        ++state.samples;
        return true;
    }
    ++state.samples;
    return false;
}

struct OwnershipDecision {
    bool write;
    const char* reason;
};

// Only a managed app that Aether already answers as owned gets its flag set.
// Steam-owned and family-shared cards keep whatever the client decided.
inline OwnershipDecision DecideOwnershipWrite(bool managed, bool genuinelyOwned, bool familyShared, bool probeLatched,
                                              std::uint32_t currentFlags) noexcept {
    if (!managed)
        return {false, "not-managed"};
    if (genuinelyOwned)
        return {false, "owned-by-steam"};
    if (familyShared)
        return {false, "family-shared"};
    if (!probeLatched)
        return {false, "probe-unproven"};
    if ((currentFlags & static_cast<std::uint32_t>(steam::AppOwnershipFlags::Owned)) != 0)
        return {false, "already-owned"};
    return {true, "owned"};
}

inline std::uint32_t ApplyOwned(std::uint32_t flags) noexcept {
    return flags | static_cast<std::uint32_t>(steam::AppOwnershipFlags::Owned);
}

} // namespace ac::hooks::steamui::policy
