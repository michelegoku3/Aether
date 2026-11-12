#include "pch.h"
#include "hooks/steamui/AppRecheckPolicy.h"

#include <algorithm>

namespace ac::hooks::steamui::policy {
namespace {

// The Lua writer has no ordering contract, so both sides of a comparison go
// through the same normalization. Two rows on one depot mean the producer broke
// the contract; the first row wins, so the verdict stays deterministic instead
// of depending on how the sort ordered the tie.
PinSet Normalize(PinSet pins) {
    std::stable_sort(pins.begin(), pins.end(),
                     [](const PinRow& left, const PinRow& right) { return left.depot < right.depot; });
    pins.erase(std::unique(pins.begin(), pins.end(),
                           [](const PinRow& left, const PinRow& right) { return left.depot == right.depot; }),
               pins.end());
    return pins;
}

} // namespace

std::vector<steam::AppId> SelectRecheck(const std::vector<PinChange>& changes) {
    std::vector<steam::AppId> apps;
    apps.reserve(changes.size());
    for (const auto& change : changes) {
        if (Normalize(change.before) == Normalize(change.after))
            continue;
        apps.push_back(change.app);
    }
    // Sorted and unique: the caller forwards this to the SteamUI thread, and a
    // stable order keeps two identical reloads from producing different work.
    std::sort(apps.begin(), apps.end());
    apps.erase(std::unique(apps.begin(), apps.end()), apps.end());
    return apps;
}

bool RecheckQueue::Push(steam::AppId app) {
    if (entries_.size() >= kCapacity)
        return false;
    if (!inFlight_.insert(app).second)
        return false;
    entries_.push_back(app);
    return true;
}

bool RecheckQueue::Pop(steam::AppId& app) {
    if (entries_.empty())
        return false;
    app = entries_.front();
    entries_.pop_front();
    inFlight_.erase(app);
    return true;
}

std::size_t RecheckQueue::Size() const { return entries_.size(); }

} // namespace ac::hooks::steamui::policy
