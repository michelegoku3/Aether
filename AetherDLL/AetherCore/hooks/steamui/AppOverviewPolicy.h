#pragma once

#include <cstddef>
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

// Capacity of the two tracking sets (ids Aether stamped, ids a removal
// retired). Bounded so a pathological config cannot grow them without limit;
// a refusal is reported, never silent. 256 is far above the number of cards a
// library rebuild touches in one session.
inline constexpr std::size_t kMaxTrackedApps = 256;

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

// What the card needs: nothing, add Aether's owned bit, or take it back.
//
// `Clear` exists because the bit is written into the client's own object, which
// caches it: when the `.lua` that justified it goes away, nothing in Steam
// recomputes the field, so the card would stay in the library until a restart.
// The bit is only ever taken back from an id Aether stamped (`stamped`) after an
// explicit removal retired it (`retired`), never because the app merely stopped
// being managed: HasDepot is false for a genuine license too.
enum class OwnershipAction {
    None,
    Stamp,
    Clear,
};

struct OwnershipDecision {
    OwnershipAction action = OwnershipAction::None;
    const char* reason = "";
};

// Everything the decision needs, in one place, so the call site cannot forget
// one condition. `managed` is luadata::HasDepot: configured by a script AND not
// genuinely owned by Steam. `sampleOk` is this card's InspectOverview verdict:
// a rejected card is never written even if the probe latched earlier.
//
// `trackingFull` is the stamp set at capacity: writing a bit that cannot be
// remembered would make it unclearable, so the write is refused instead.
struct WriteInputs {
    bool managed = false;
    bool familyShared = false;
    bool retired = false;
    bool stamped = false;
    bool trackingFull = false;
    bool sampleOk = false;
    bool probeLatched = false;
    bool layoutTrusted = false; // the ABI table did not contradict the layout
    std::uint32_t currentFlags = 0;
};

inline OwnershipDecision DecideOwnershipWrite(const WriteInputs& in) noexcept {
    // Layout evidence first: a Clear is a write into the same object and needs
    // the same proof as a Stamp.
    if (!in.sampleOk)
        return {OwnershipAction::None, "sample-rejected"};
    if (!in.probeLatched)
        return {OwnershipAction::None, "probe-unproven"};
    if (!in.layoutTrusted)
        return {OwnershipAction::None, "layout-contradicted"};
    // A real family share (or the cosmetic banner) is Valve's or display-only;
    // both keep the bit the client set.
    if (in.familyShared)
        return {OwnershipAction::None, "family-shared"};

    const bool owned = (in.currentFlags & static_cast<std::uint32_t>(steam::AppOwnershipFlags::Owned)) != 0;
    if (in.managed) {
        if (owned)
            return {OwnershipAction::None, "already-owned"};
        if (in.trackingFull)
            return {OwnershipAction::None, "tracking-full"};
        return {OwnershipAction::Stamp, "owned"};
    }
    if (in.retired && in.stamped) {
        if (!owned)
            return {OwnershipAction::None, "already-clear"};
        return {OwnershipAction::Clear, "retired"};
    }
    return {OwnershipAction::None, "not-managed"};
}

inline std::uint32_t ApplyOwned(std::uint32_t flags) noexcept {
    return flags | static_cast<std::uint32_t>(steam::AppOwnershipFlags::Owned);
}

inline std::uint32_t ApplyCleared(std::uint32_t flags) noexcept {
    return flags & ~static_cast<std::uint32_t>(steam::AppOwnershipFlags::Owned);
}

} // namespace ac::hooks::steamui::policy
