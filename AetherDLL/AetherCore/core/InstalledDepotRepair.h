#pragma once

#include <cstddef>
#include <cstdint>
#include <functional>
#include <string>
#include <string_view>
#include <vector>

#include "core/AppManifestText.h"

// ---------------------------------------------------------------------------
// `InstalledDepots` repair: keep the ACF's installed GID backed by a real file.
//
// Steam computes the delta of every download and update from the manifest GID
// recorded in `steamapps/appmanifest_<app>.acf`:
//
//   InstalledDepots { "<depot>" { "manifest" "<gid>" "size" "<bytes>" } }
//
// `contentupdatecontext.cpp` / `depotdownloadmgr.cpp` ask the CM for that exact
// GID before anything else. When no `<depot>_<gid>.manifest` exists in
// `depotcache`, the request is denied (the account does not own the app), the
// download aborts with `Access Denied` and Steam reports "No connection" — even
// though the content is on disk and another local manifest describes it.
//
// That state is reachable: AetherDesk used to write the pinned GID into
// `InstalledDepots` while applying a version, its completeness gate only proved
// the manifest file was staged, and Steam later dropped that file (cache clean,
// uninstall, reinstall under another build). The content never moved, so the
// ACF's `size` still describes the file that *is* on disk.
//
// This pass therefore restores the GID of a manifest the depot really has,
// picking the one whose own content size equals the size Steam recorded (the
// file the installed content came from) and otherwise the newest local file:
// Steam re-verifies every chunk, so an extra verification is harmless while a
// GID that cannot be fetched is fatal.
//
// Runs once per process, at boot, for managed apps only (Steam reads app state
// later in the same start). It never touches build fields, never creates
// manifests, and does nothing when the declared GID already resolves or when
// the depot has no local manifest at all. Unwritable manifests are logged and
// left for the next Steam start: no retry loop, no background polling.
//
// Header-only policy (no filesystem, no logging, no g_state) so the decision
// matrix is unit-testable; the boot pass lives in InstalledDepotRepair.cpp.
// ---------------------------------------------------------------------------
namespace ac::appmanifest {

struct DepotEntry {
    std::uint32_t depotId = 0;
    std::size_t manifestLine = 0; // index into the split lines
    std::string manifestGid;
    bool hasSize = false;
    std::size_t sizeLine = 0;
    std::uint64_t size = 0;
};

// One `<depot>_<gid>.manifest` already on disk.
struct LocalManifest {
    std::string gid;
    bool hasContentSize = false;
    std::uint64_t contentSize = 0;
    // Opaque recency key (file write time); higher is newer.
    std::uint64_t modified = 0;
};

struct Repair {
    std::uint32_t depotId = 0;
    std::string declaredGid;
    std::string restoredGid;
};

struct RepairPlan {
    // The ACF body to publish. Equal to the input when `repairs` is empty.
    std::string content;
    std::vector<Repair> repairs;
};

// A manifest container split into lines without their endings, so a plan that
// changes nothing re-emits the file byte-for-byte.
struct SplitContent {
    std::vector<std::string> lines;
    std::vector<bool> crlf;
    bool trailingNewline = false;
};

inline SplitContent SplitContentLines(const std::string& content) {
    SplitContent split;
    std::size_t start = 0;
    while (start < content.size()) {
        const std::size_t newline = content.find('\n', start);
        const std::size_t stop = (newline == std::string::npos) ? content.size() : newline;
        std::string line = content.substr(start, stop - start);
        bool carriageReturn = false;
        if (!line.empty() && line.back() == '\r') {
            carriageReturn = true;
            line.pop_back();
        }
        split.lines.push_back(std::move(line));
        split.crlf.push_back(carriageReturn);
        if (newline == std::string::npos) {
            split.trailingNewline = false;
            break;
        }
        start = newline + 1;
        if (start == content.size()) {
            split.trailingNewline = true;
            break;
        }
    }
    return split;
}

inline std::string JoinContentLines(const SplitContent& split) {
    std::string out;
    for (std::size_t i = 0; i < split.lines.size(); ++i) {
        out += split.lines[i];
        if (i + 1 < split.lines.size() || split.trailingNewline)
            out += split.crlf[i] ? "\r\n" : "\n";
    }
    return out;
}

inline std::string_view TrimAscii(std::string_view text) {
    std::size_t begin = 0;
    std::size_t end = text.size();
    while (begin < end && (text[begin] == ' ' || text[begin] == '\t' || text[begin] == '\r'))
        ++begin;
    while (end > begin && (text[end - 1] == ' ' || text[end - 1] == '\t' || text[end - 1] == '\r'))
        --end;
    return text.substr(begin, end - begin);
}

inline std::uint32_t ParseDepotId(std::string_view text) {
    if (text.empty())
        return 0;
    std::uint64_t value = 0;
    for (const char c : text) {
        if (c < '0' || c > '9')
            return 0;
        value = value * 10 + static_cast<std::uint64_t>(c - '0');
        if (value > 0xFFFFFFFFull)
            return 0;
    }
    return static_cast<std::uint32_t>(value);
}

inline std::uint64_t ParseGid(std::string_view text) {
    if (text.empty())
        return 0;
    std::uint64_t value = 0;
    for (const char c : text) {
        if (c < '0' || c > '9')
            return 0;
        if (value > (UINT64_MAX - static_cast<std::uint64_t>(c - '0')) / 10)
            return 0;
        value = value * 10 + static_cast<std::uint64_t>(c - '0');
    }
    return value;
}

// Every depot entry inside `InstalledDepots`, with the line numbers of the
// `manifest`/`size` values so a repair can rewrite them in place.
inline std::vector<DepotEntry> ParseInstalledDepots(const std::vector<std::string>& lines) {
    std::vector<DepotEntry> entries;
    int depth = 0;
    int installedDepth = -1; // depth of the InstalledDepots body
    bool expectInstalled = false;
    std::uint32_t expectedDepot = 0;
    int depotDepth = -1;
    bool inDepot = false;
    DepotEntry current;

    for (std::size_t index = 0; index < lines.size(); ++index) {
        const std::string& line = lines[index];
        std::string key;
        std::string value;
        const vdf::LineKind kind = vdf::ParseLine(line, key, value);
        const std::string_view trimmed = TrimAscii(line);

        if (kind == vdf::LineKind::SectionKey) {
            if (installedDepth < 0) {
                if (depth == 1 && ac::strings::EqualsIgnoreCase(key, "InstalledDepots"))
                    expectInstalled = true;
            } else if (depth == installedDepth) {
                const std::uint32_t depotId = ParseDepotId(key);
                if (depotId != 0)
                    expectedDepot = depotId;
            }
            continue;
        }

        if (trimmed == "{") {
            ++depth;
            if (expectInstalled) {
                installedDepth = depth;
                expectInstalled = false;
            } else if (expectedDepot != 0 && installedDepth >= 0 && !inDepot) {
                depotDepth = depth;
                current = DepotEntry{};
                current.depotId = expectedDepot;
                expectedDepot = 0;
                inDepot = true;
            }
            continue;
        }
        if (trimmed == "}") {
            if (inDepot && depth == depotDepth) {
                if (ParseGid(current.manifestGid) != 0)
                    entries.push_back(current);
                inDepot = false;
                current = DepotEntry{};
            }
            if (installedDepth >= 0 && depth == installedDepth)
                installedDepth = -1;
            --depth;
            continue;
        }
        if (kind == vdf::LineKind::KeyValue && inDepot && depth == depotDepth) {
            if (ac::strings::EqualsIgnoreCase(key, "manifest")) {
                current.manifestLine = index;
                current.manifestGid = value;
            } else if (ac::strings::EqualsIgnoreCase(key, "size")) {
                current.hasSize = true;
                current.sizeLine = index;
                current.size = ParseGid(value);
            }
        }
    }
    return entries;
}

// Picks the local manifest that should replace `entry`'s declared GID. False
// when the declared GID already resolves or there is nothing to choose from.
inline bool ChooseLocalManifest(const DepotEntry& entry, const std::vector<LocalManifest>& candidates,
                                LocalManifest& chosen) {
    if (candidates.empty())
        return false;
    for (const LocalManifest& candidate : candidates) {
        if (candidate.gid == entry.manifestGid)
            return false;
    }

    if (entry.hasSize && entry.size > 0) {
        for (const LocalManifest& candidate : candidates) {
            if (candidate.hasContentSize && candidate.contentSize == entry.size) {
                chosen = candidate;
                return true;
            }
        }
    }

    // Newest wins; the GID breaks ties so the choice never depends on the
    // directory enumeration order.
    const LocalManifest* best = &candidates.front();
    for (const LocalManifest& candidate : candidates) {
        if (candidate.modified > best->modified ||
            (candidate.modified == best->modified && candidate.gid > best->gid)) {
            best = &candidate;
        }
    }
    chosen = *best;
    return true;
}

inline bool ReplaceLineValue(std::string& line, const std::string& value) {
    std::size_t begin = 0;
    std::size_t length = 0;
    if (!FindValueSpan(line, begin, length))
        return false;
    line.replace(begin, length, value);
    return true;
}

// `isBacked` answers "does <depot>_<gid>.manifest exist and is non-empty";
// `candidates` lists what the depot does have. Both are callbacks so the
// decision matrix is testable without a filesystem.
inline RepairPlan
PlanInstalledDepotRepairs(const std::string& content,
                          const std::function<bool(std::uint32_t, const std::string&)>& isBacked,
                          const std::function<std::vector<LocalManifest>(std::uint32_t)>& candidates) {
    RepairPlan plan;
    plan.content = content;
    SplitContent split = SplitContentLines(content);
    for (const DepotEntry& entry : ParseInstalledDepots(split.lines)) {
        if (isBacked && isBacked(entry.depotId, entry.manifestGid))
            continue;

        LocalManifest chosen;
        const std::vector<LocalManifest> available =
            candidates ? candidates(entry.depotId) : std::vector<LocalManifest>{};
        if (!ChooseLocalManifest(entry, available, chosen))
            continue;

        if (!ReplaceLineValue(split.lines[entry.manifestLine], chosen.gid))
            continue;
        if (entry.hasSize && chosen.hasContentSize)
            ReplaceLineValue(split.lines[entry.sizeLine], std::to_string(chosen.contentSize));

        plan.repairs.push_back(Repair{entry.depotId, entry.manifestGid, chosen.gid});
    }
    if (!plan.repairs.empty())
        plan.content = JoinContentLines(split);
    return plan;
}

struct RepairStats {
    std::size_t scanned = 0;  // managed ACFs read
    std::size_t repaired = 0; // depot entries restored
    std::size_t errors = 0;
};

// One pass over every managed app's ACF in every Steam library. Never throws.
RepairStats ApplyInstalledDepotRepairs();

// Queues the single boot-time pass on the worker thread. Must be called once
// per process, before Steam builds this session's app list.
void ApplyInstalledDepotRepairsAtStartup();

} // namespace ac::appmanifest
