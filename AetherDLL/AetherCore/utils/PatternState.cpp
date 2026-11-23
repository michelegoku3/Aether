#include "pch.h"
#include "utils/PatternState.h"

#include <mutex>
#include <utility>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "utils/PatternDownloader.h"
#include "utils/PatternStateParse.h"

namespace ac::pattern {
namespace {

constexpr const char* kModule = "PatternEngine";
// One small GET per process, on whichever thread asks first, over one endpoint.
// The pattern threads wait for it before hooks install, so the cap is short: an
// unreachable index costs this wait once per launch, then the cached tables are
// served as before.
constexpr int kIndexTimeoutSec = 3;

struct Snapshot {
    StateIndex entries;
    std::string source = "unavailable";
    std::size_t tables = 0;
};

std::once_flag g_load;
Snapshot g_snapshot;

void PublishSource() {
    std::lock_guard lock(g_state.statusMetadataMutex);
    g_state.patternIndexSource = g_snapshot.source;
}

void Load() {
    const auto settings = Settings::Snapshot();
    if (!settings->patternMirror.empty()) {
        // The mirror is the user's chosen source of truth and need not carry
        // the index; the provenance policy already keeps that table fresh.
        g_snapshot.source = "mirror";
        AC_LOG_INFO(kModule, "Custom pattern mirror configured; the published generation index is ignored.");
        PublishSource();
        return;
    }

    std::string body;
    std::string source;
    if (!downloader::DownloadState(body, &source, kIndexTimeoutSec)) {
        AC_LOG_INFO(kModule, "Pattern index unavailable; cached tables stay as they are.");
        PublishSource();
        return;
    }

    StateParse parsed = ParseState(body);
    if (parsed.modules.empty()) {
        // A 200 with nothing usable is not an index: report it as unavailable so
        // status.json does not claim a source that described nothing.
        AC_LOG_WARN(kModule, "Pattern index from '%s' carries no entry; ignoring it.", source.c_str());
        PublishSource();
        return;
    }
    for (const auto& [_, generations] : parsed.modules)
        g_snapshot.tables += generations.size();
    g_snapshot.entries = std::move(parsed.modules);
    g_snapshot.source = source.empty() ? "unavailable" : source;
    if (parsed.rejected > 0) {
        AC_LOG_WARN(kModule, "Pattern index: %zu line(s) could not be read and were ignored.", parsed.rejected);
    }
    AC_LOG_INFO(kModule, "Pattern index from '%s': %zu table(s) in %zu module(s).", g_snapshot.source.c_str(),
                g_snapshot.tables, g_snapshot.entries.size());
    PublishSource();
}

} // namespace

std::uint64_t RemoteVersion(const std::string& module, const std::string& sha) {
    std::call_once(g_load, Load);
    return Lookup(g_snapshot.entries, module, sha);
}

} // namespace ac::pattern
