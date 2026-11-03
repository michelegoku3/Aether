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

// Everything the decision needs, in one place, so the call site cannot forget
// one condition. `managed` is luadata::HasDepot: configured by a script AND not
// genuinely owned by Steam. `sampleOk` is this card's InspectOverview verdict:
// a rejected card is never written even if the probe latched earlier.
struct WriteInputs {
    bool managed = false;
    bool familyShared = false;
    bool sampleOk = false;
    bool probeLatched = false;
    bool layoutTrusted = false; // the ABI table did not contradict the layout
    std::uint32_t currentFlags = 0;
};

inline OwnershipDecision DecideOwnershipWrite(const WriteInputs& in) noexcept {
    if (!in.managed)
        return {false, "not-managed"};
    if (in.familyShared)
        return {false, "family-shared"};
    if (!in.sampleOk)
        return {false, "sample-rejected"};
    if (!in.probeLatched)
        return {false, "probe-unproven"};
    if (!in.layoutTrusted)
        return {false, "layout-contradicted"};
    if ((in.currentFlags & static_cast<std::uint32_t>(steam::AppOwnershipFlags::Owned)) != 0)
        return {false, "already-owned"};
    return {true, "owned"};
}

inline std::uint32_t ApplyOwned(std::uint32_t flags) noexcept {
    return flags | static_cast<std::uint32_t>(steam::AppOwnershipFlags::Owned);
}

} // namespace ac::hooks::steamui::policy
