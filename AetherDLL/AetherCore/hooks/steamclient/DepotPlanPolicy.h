#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <string_view>
#include <unordered_map>
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
// The repair has two sources because the two states need different rows. A
// not-installed app has no record of its content: the `.lua` names its depots,
// and AppInfo separates them from depots of other apps. An installed app has
// one - its appmanifest's `InstalledDepots` - and that record is what keeps the
// content on disk: Steam removes a depot its stored plan leaves out and deletes
// the files only that depot carried. Steam writes the ACF at the start of an
// update, so a package-0 app can look installed while its plan is still empty;
// restoring the recorded depots then is the difference between a game that
// launches and a missing content file.
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
    // A managed app whose plan still has no own rows after the refill: the
    // AppInfo is mid-swap, the ACF is unreadable, or every recorded depot is
    // already planned. Refusing makes the caller keep the depot config it
    // already stored.
    return answer.ownRows == 0 ? PlanVerdict::Refuse : PlanVerdict::Keep;
}

// A depot the app's `.lua` names: what the app is made of, independent of
// Steam's licensing answer.
struct PinnedDepot {
    steam::AppId depotId = 0;
    std::uint64_t gid = 0;
    std::uint64_t size = 0;
};

// The size a refilled row carries: the pin's or ACF's own size, and AppInfo's
// public size only when neither has one. A zero stays zero rather than a guess.
constexpr std::uint64_t RowSize(const PinnedDepot& row, std::uint64_t appInfoSize) {
    return row.size != 0 ? row.size : appInfoSize;
}

// True when a depot's AppInfo `config/oslist` lets it run on `host`.
//
// The key is a comma-separated platform list ("windows", "windows,linux"). A
// `.lua` lists every depot the store has, including the ones only another
// platform runs, and two platforms can declare the same file path: planning
// both puts two builds in one install folder and the shared path ends up as
// whichever depot committed last. An absent or empty list places no
// restriction, so the depot applies.
constexpr char FoldAscii(char c) { return (c >= 'A' && c <= 'Z') ? static_cast<char>(c - 'A' + 'a') : c; }

constexpr bool OsListAllowsHost(std::string_view osList, std::string_view host) noexcept {
    if (osList.empty() || host.empty())
        return true;
    std::size_t begin = 0;
    while (begin < osList.size()) {
        std::size_t end = osList.find(',', begin);
        if (end == std::string_view::npos)
            end = osList.size();
        std::size_t first = begin;
        std::size_t last = end;
        while (first < last && (osList[first] == ' ' || osList[first] == '\t'))
            ++first;
        while (last > first && (osList[last - 1] == ' ' || osList[last - 1] == '\t'))
            --last;
        if (last - first == host.size()) {
            bool same = true;
            for (std::size_t i = 0; i < host.size(); ++i) {
                if (FoldAscii(osList[first + i]) != FoldAscii(host[i])) {
                    same = false;
                    break;
                }
            }
            if (same)
                return true;
        }
        begin = end + 1;
    }
    return false;
}

// Whether a pinned depot Steam answered with as an installed-only row has to
// become a target row.
//
// Steam carries a depot that is already on disk over as a row flagged
// `notNewTarget`, and its update job compares target rows only: a plan whose own
// depots are all installed-only makes every pin change a zero-file update that
// still mounts and records the pinned gid as installed. A pin that names a gid
// the appmanifest does not is that target, so the flag is cleared. A pin that is
// already installed, and a depot the appmanifest does not record, leave the row
// the way Steam shaped it.
constexpr bool PromotePinnedTarget(std::uint64_t pinnedGid, std::uint64_t recordedGid, bool notNewTarget) {
    return notNewTarget && pinnedGid != 0 && recordedGid != 0 && pinnedGid != recordedGid;
}

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

// A depot the app's own appmanifest records as installed: the content Steam
// accounts for on disk.
struct RecordedDepot {
    steam::AppId depotId = 0;
    std::uint64_t gid = 0;
    std::uint64_t size = 0;
};

// The rows an installed app's plan is missing: every depot its ACF records and
// Steam's answer left out, at the build the `.lua` selects.
//
// The ACF lists what the client installed, so a depot it names has content on
// disk and leaving it out of the plan makes Steam remove the depot and delete
// its files. A managed package-0 app answers with no own rows even straight
// after its own commit wrote that file, which is exactly when this runs.
//
// A pin's gid and size win over the recorded ones: the `.lua` selects the build
// the user picked, and the recorded values are the fallback for an unpinned
// depot or an AppInfo that is mid-swap. Skipped: the app itself (not content),
// rows Steam already planned (its row carries the branch gid, and a second row
// would download the depot twice), repeats, and entries without a gid (nothing
// to install). Ordered by depot id so a plan built twice is identical.
inline std::vector<PinnedDepot> SelectRecordedRows(steam::AppId app, const std::vector<steam::AppId>& planned,
                                                   const std::vector<PinnedDepot>& pinned,
                                                   const std::vector<RecordedDepot>& recorded) {
    std::unordered_set<steam::AppId> plannedIds(planned.begin(), planned.end());
    plannedIds.insert(app);

    std::unordered_map<steam::AppId, const PinnedDepot*> byDepot;
    for (const PinnedDepot& pin : pinned) {
        if (pin.depotId != 0)
            byDepot.emplace(pin.depotId, &pin); // a repeated pin keeps the first
    }

    std::unordered_set<steam::AppId> seen;
    std::vector<PinnedDepot> rows;
    rows.reserve(recorded.size());
    for (const RecordedDepot& entry : recorded) {
        if (entry.depotId == 0 || plannedIds.count(entry.depotId) || !seen.insert(entry.depotId).second)
            continue;
        PinnedDepot row{entry.depotId, entry.gid, entry.size};
        if (const auto found = byDepot.find(entry.depotId); found != byDepot.end()) {
            const PinnedDepot& pin = *found->second;
            if (pin.gid != 0)
                row.gid = pin.gid;
            if (pin.size > 0)
                row.size = pin.size;
        }
        if (row.gid == 0)
            continue;
        rows.push_back(row);
    }
    std::sort(rows.begin(), rows.end(),
              [](const PinnedDepot& a, const PinnedDepot& b) { return a.depotId < b.depotId; });
    return rows;
}

// The gid an installed row has to be planned at when the `.lua` leaves its
// depot unpinned, or 0 when the row stays as it is.
//
// Steam carries an unowned depot on disk over as an installed-only row
// (`notNewTarget`) at the ACF's recorded gid instead of resolving AppInfo: an
// app whose `.lua` pins are released would otherwise compare the installed gid
// with itself and never offer the update. An unpinned installed-only row whose
// recorded gid differs from AppInfo's public gid moves to `publicGid`; a
// non-installed-only row (`!notNewTarget`, e.g. a branch build Steam resolved
// itself), a row whose gid differs from `recordedGid`, a locked depot
// (`pinned`), or a missing/unchanged public gid stay untouched.
constexpr std::uint64_t PublicTargetFor(std::uint64_t rowGid, std::uint64_t recordedGid, std::uint64_t publicGid,
                                        bool notNewTarget, bool pinned) noexcept {
    if (pinned || rowGid != recordedGid)
        return 0;
    return PromotePinnedTarget(publicGid, recordedGid, notNewTarget) ? publicGid : 0;
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

// Where the rows a lost plan is refilled from come from.
enum class RefillSource {
    // Nothing to put back: no `.lua` depot to plan, no ACF record to restore,
    // or a real family share whose plan is Steam's own answer.
    None,
    // A not-installed app: the `.lua` pins describe its content.
    Pins,
    // An installed app: the depots its appmanifest records are what is on disk.
    Recorded,
};

// Whether the plan needs rows, and from where.
//
// A not-installed app has no record of its content, and the plan is the only
// thing that can size the install dialog, so its depots come from the `.lua`
// pins. An installed app is restored from its own appmanifest instead: the file
// records every depot whose content is on disk, each language pack included,
// while the `.lua` lists every depot the store has. Filling an installed app's
// plan from the `.lua` would mark each language pack as content that must exist
// and the client would download the difference; filling it from the ACF keeps
// exactly what is there.
constexpr RefillSource DecideRefill(bool installed, std::uint32_t pinnedRows, bool realFamilyShare) {
    // A real family share is licensed by Steam: its answer already plans what
    // the account may use, and a row added here would claim content it does not.
    if (realFamilyShare)
        return RefillSource::None;
    if (installed)
        return RefillSource::Recorded;
    return pinnedRows > 0 ? RefillSource::Pins : RefillSource::None;
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
