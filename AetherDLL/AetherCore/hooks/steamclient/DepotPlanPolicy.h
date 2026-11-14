#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <unordered_set>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Depot plan policy: what Aether does with the depot rows Steam built for an
// app, and which rows it puts back.
//
// Steam derives an app's plan from its AppInfo *and its license answer*. Two
// different failures leave a managed app with no rows of its own:
//
//  - a PICS refresh drops the old depot set before the new one lands, so the
//    call answers with zero depots for a moment;
//  - the client does not license the content of an app that only package 0
//    owns, so the plan keeps just the shared redistributable rows another app
//    contributes.
//
// The caller of BuildDepotDependency stores whatever comes back, and a stored
// plan without the app's own depots is an install with nothing to download: it
// reports 0 B (or only the redistributable rows), "completes" without writing a
// file, and still stamps the app Fully Installed - so the library shows no size
// and PLAY looks for an executable nobody downloaded. The `.lua` names the app's
// depots, so the second case is repaired before the answer is stored; if nothing
// can be put back, the answer is refused and the caller keeps the depot config
// Steam already stored.
//
// The decisions and the arithmetic behind the repair are pure reads with no
// logger and no shared state, so the unit suite exercises the real thing. Only
// the app's own vector is an input: the shared/dependency vector holds rows
// belonging to other apps (redistributables, launcher redirects), so one of those
// rows cannot make an empty own list look planned.
// ---------------------------------------------------------------------------
namespace ac::hooks::steamclient::policy {

// What Steam's BuildDepotDependency handed back for one app.
struct PlanAnswer {
    // Rows in the app's own depot vector.
    std::uint32_t ownRows = 0;
    // True when a `.lua` names the app, i.e. Aether is why Steam plans it at all.
    bool managed = false;
    // What the original call returned.
    bool steamOk = true;
};

enum class PlanVerdict {
    // Store the answer.
    Keep,
    // Refuse the answer so Steam keeps the depot config it already stored.
    Refuse,
};

constexpr PlanVerdict DecidePlan(PlanAnswer answer) {
    // Steam already failed the call: its caller keeps the previous config on
    // that path, so there is nothing to refuse and nothing to log.
    if (!answer.steamOk)
        return PlanVerdict::Keep;
    // No `.lua` names the app, so an empty plan is Steam's own answer for it.
    // Refusing would leave a config the user cannot clear from Aether.
    if (!answer.managed)
        return PlanVerdict::Keep;
    // A managed app whose plan came back empty is mid-AppInfo-swap. Refusing
    // makes the caller keep the depot config it already stored.
    return answer.ownRows == 0 ? PlanVerdict::Refuse : PlanVerdict::Keep;
}

// A depot the app's `.lua` names: what the app is made of, independent of
// Steam's licensing answer.
struct PinnedDepot {
    steam::AppId depotId = 0;
    std::uint64_t gid = 0;
    std::uint64_t size = 0;
};

// The rows to append to a plan Steam answered with, given the depots the `.lua`
// names for the app and the depot ids Steam already planned.
//
// Current builds answer with few or no own rows for a managed app whose content
// the client does not license; the plan is what the install dialog sizes and what
// the downloader walks, so an app that is planned empty reports 0 B and
// "installs" nothing. The `.lua` names the app's depots, so they fill it.
//
// Skipped: the app itself (not content), depots without a gid (no manifest to
// install), and depots Steam already planned (its row carries the branch gid,
// and a second row would download the depot twice). Repeats keep the first pin,
// and the result is ordered by depot id so a plan built twice is identical.
inline std::vector<PinnedDepot> SelectRefillRows(steam::AppId app, const std::vector<steam::AppId>& planned,
                                                 const std::vector<PinnedDepot>& pinned) {
    std::unordered_set<steam::AppId> plannedIds(planned.begin(), planned.end());
    plannedIds.insert(app);

    std::unordered_set<steam::AppId> seen;
    std::vector<PinnedDepot> rows;
    rows.reserve(pinned.size());
    for (const PinnedDepot& depot : pinned) {
        if (depot.depotId == 0 || depot.gid == 0)
            continue;
        if (plannedIds.count(depot.depotId) || !seen.insert(depot.depotId).second)
            continue;
        rows.push_back(depot);
    }
    std::sort(rows.begin(), rows.end(),
              [](const PinnedDepot& a, const PinnedDepot& b) { return a.depotId < b.depotId; });
    return rows;
}

// ---------------------------------------------------------------------------
// Room for those rows.
//
// A plan is a CUtlVector of 32-byte DepotEntry rows, but the grow helper the
// pattern table names is the uint32 instantiation: its copy loop shifts by two
// (`shl rdx, 2`) and it hands the allocator `mov r9d, 4`, so it reserves four
// bytes per requested element whatever the vector holds. Asking it for N rows
// reserves N bytes per row and the append writes 32 - past the block, which
// surfaces later, somewhere else. A request is therefore expressed in the
// helper's unit and checked in bytes.
// ---------------------------------------------------------------------------
constexpr std::uint32_t kGrowElementBytes = 4;
constexpr std::uint32_t kPlanRowBytes = 0x20;

constexpr std::uint32_t GrowSlotsFor(std::uint32_t rows) { return rows * (kPlanRowBytes / kGrowElementBytes); }

// True when `slots` uint32 elements hold `rows` plan rows.
constexpr bool GrowCoversRows(std::uint32_t slots, std::uint32_t rows) {
    return slots * kGrowElementBytes >= rows * kPlanRowBytes;
}

enum class RefillVerdict {
    // Store Steam's answer as built.
    Skip,
    // Add the app's depots to the plan.
    Refill,
};

// Whether the plan needs the `.lua`'s depots.
//
// An installed app is Steam's to plan. Its appmanifest already records what is
// on disk, each language pack included, and Steam keeps the plan in step with
// it: the `.lua` lists every depot the store has, so filling an installed app's
// plan from it marks each language pack as content that must exist, and the
// client downloads the difference. Before the app is installed there is no such
// record, and the plan is the only thing that can size the dialog at all.
constexpr RefillVerdict DecideRefill(bool installed, std::uint32_t pinnedRows) {
    if (installed || pinnedRows == 0)
        return RefillVerdict::Skip;
    return RefillVerdict::Refill;
}

// AppInfo answers a manifest's gid and size as a decimal string. A result that
// does not parse is no value at all, never a truncated number that would size the
// install wrong.
inline std::uint64_t ParseSize(const char* text, std::size_t len) {
    std::uint64_t value = 0;
    for (std::size_t i = 0; i < len; ++i) {
        const char c = text[i];
        if (c < '0' || c > '9')
            break;
        value = value * 10 + static_cast<std::uint64_t>(c - '0');
    }
    return value;
}

} // namespace ac::hooks::steamclient::policy
