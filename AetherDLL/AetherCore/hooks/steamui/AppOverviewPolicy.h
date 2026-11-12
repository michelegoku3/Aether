#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <unordered_set>
#include <vector>

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
// The bit is only ever taken back from an id Aether provided (`tracked`) after an
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
// `tracked` is "Aether provided this app on an earlier card fill in this
// process". The owned bit is often already set by the client, because package 0
// makes Steam answer the app as licensed; that bit is still Aether's to take
// back when the `.lua` goes away, so the set records management, not authorship.
// `genuinelyOwned` keeps a purchased app's bit.
//
// `trackingFull` is the set at capacity: a management this process cannot
// remember would leave an unclearable bit, so the write is refused instead.
struct WriteInputs {
    bool managed = false;
    bool genuinelyOwned = false;
    bool familyShared = false;
    bool retired = false;
    bool tracked = false;
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

    const bool owned = (in.currentFlags & static_cast<std::uint32_t>(steam::AppOwnershipFlags::Owned)) != 0;
    if (in.managed) {
        // A real family share (or the cosmetic banner) keeps the bit the client
        // set: the card is displayed through the family answer, not through us.
        if (in.familyShared)
            return {OwnershipAction::None, "family-shared"};
        if (owned)
            return {OwnershipAction::None, "already-owned"};
        if (in.trackingFull)
            return {OwnershipAction::None, "tracking-full"};
        return {OwnershipAction::Stamp, "owned"};
    }
    // A licence Steam granted after we started managing the app is not ours to
    // take away.
    if (in.genuinelyOwned)
        return {OwnershipAction::None, "owned-by-steam"};
    if (in.familyShared)
        return {OwnershipAction::None, "family-shared"};
    if (in.retired && in.tracked) {
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

// Whether a hidden card is due back. Only a card Aether took back, and only once
// its `.lua` is configured again: the lookup that restores it writes into the
// client's object, so it must never run for an app Aether never hid.
inline bool ShouldRestoreCard(bool hidden, bool managed) noexcept { return hidden && managed; }

// A removed file's library roots that no script configures any more.
//
// `beforeRoots` is every root the removed file advertised (the keys of its pin
// snapshot); `configuredAfter` is the global configured set once the whole
// change batch has been applied, not a per-file answer. That is what makes an
// atomic replace safe: a save lands as a temporary file plus a rename, so
// the new file is already parsed when this runs and a root that came back is
// still in the set.
inline std::vector<steam::AppId> RetiredRoots(std::vector<steam::AppId> beforeRoots,
                                              const std::unordered_set<steam::AppId>& configuredAfter) {
    std::sort(beforeRoots.begin(), beforeRoots.end());
    beforeRoots.erase(std::unique(beforeRoots.begin(), beforeRoots.end()), beforeRoots.end());
    beforeRoots.erase(std::remove_if(beforeRoots.begin(), beforeRoots.end(),
                                     [&configuredAfter](steam::AppId app) { return configuredAfter.count(app) != 0; }),
                      beforeRoots.end());
    return beforeRoots;
}

} // namespace ac::hooks::steamui::policy
