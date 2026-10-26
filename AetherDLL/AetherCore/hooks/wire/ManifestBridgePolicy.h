#pragma once

#include <cstddef>
#include <cstdint>
#include <deque>
#include <optional>
#include <unordered_map>

// ---------------------------------------------------------------------------
// Manifest request-code bridge: the decision half.
//
// Steam asks the CM for a `ContentServerDirectory.GetManifestRequestCode` code
// before it will download a depot whose manifest it does not consider owned.
// Aether obtains the exact manifest locally (depotcache or the AetherData
// backup) or through the Hubcap pipeline; once it is on disk the right answer
// is code 0. The bridge therefore answers only two questions:
//
//   * did this session send a matching request (so the reply is ours to edit)?
//   * is the exact manifest on disk right now (never wait: this runs on Steam's
//     network thread, and Steam retries on its own)?
//
// Kept header-only and free of protobuf/Steam state so the correlation table and
// the decision are unit-testable.
// ---------------------------------------------------------------------------
namespace ac::hooks::manifestbridge {

struct PendingRequest {
    std::uint32_t appId = 0;
    std::uint32_t depotId = 0;
    std::uint64_t gid = 0;
};

// jobid -> request. The CM answers every request, so entries are normally
// consumed by their response; the cap only bounds abandoned jobs (cancelled
// downloads, dropped connections) and evicts oldest first.
class RequestTable {
  public:
    static constexpr std::size_t kMaxEntries = 256;

    void Record(std::uint64_t jobId, const PendingRequest& request) {
        if (entries_.find(jobId) != entries_.end()) {
            entries_[jobId] = request;
            return;
        }
        while (entries_.size() >= kMaxEntries && !order_.empty()) {
            entries_.erase(order_.front());
            order_.pop_front();
        }
        entries_.emplace(jobId, request);
        order_.push_back(jobId);
    }

    std::optional<PendingRequest> Take(std::uint64_t jobId) {
        const auto it = entries_.find(jobId);
        if (it == entries_.end())
            return std::nullopt;
        const PendingRequest request = it->second;
        entries_.erase(it);
        return request;
    }

    std::size_t Size() const { return entries_.size(); }

    void Clear() {
        entries_.clear();
        order_.clear();
    }

  private:
    std::unordered_map<std::uint64_t, PendingRequest> entries_;
    std::deque<std::uint64_t> order_;
};

enum class BridgeAction {
    PassThrough, // leave Steam's reply (typically AccessDenied) alone
    Satisfy,     // answer OK with request code 0
};

inline BridgeAction Decide(bool hadRequest, bool localManifest) {
    return hadRequest && localManifest ? BridgeAction::Satisfy : BridgeAction::PassThrough;
}

} // namespace ac::hooks::manifestbridge
