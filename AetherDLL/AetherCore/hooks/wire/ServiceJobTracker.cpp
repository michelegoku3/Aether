#include "pch.h"
#include "hooks/wire/ServiceJobTracker.h"

#include <deque>
#include <mutex>
#include <unordered_map>
#include <utility>

#include "steam_messages.pb.h"

namespace ac::hooks {
namespace {

// [DIAG]/flight-recorder: jobid_source -> target_job_name correlation on
// 151 sends, resolved jobid_target -> name on 147 recvs (responses carry
// no service name, only the request's job id).
std::mutex s_jobNameMutex;
std::unordered_map<std::uint64_t, std::string> s_jobNames;
// Insertion order: eviction at cap drops the OLDEST entry (before:
// erase(begin()) on unordered_map = arbitrary element).
std::deque<std::uint64_t> s_jobNameOrder;

} // namespace

bool ParseServiceRequest(const WireFrame& f, ServiceRequest& out) {
    CMsgProtoBufHeader hdr;
    if (!hdr.ParseFromArray(f.header, static_cast<int>(f.headerLen)))
        return false;
    if (hdr.has_target_job_name()) {
        out.name = hdr.target_job_name();
        out.hasName = true;
    }
    if (hdr.has_jobid_source()) {
        out.source = hdr.jobid_source();
        out.hasSource = true;
    }
    return out.hasName || out.hasSource;
}

bool ParseServiceResponse(const WireFrame& f, ServiceResponse& out) {
    CMsgProtoBufHeader hdr;
    if (!hdr.ParseFromArray(f.header, static_cast<int>(f.headerLen)))
        return false;
    if (hdr.has_target_job_name()) {
        out.name = hdr.target_job_name();
        out.hasName = true;
    }
    if (hdr.has_jobid_target()) {
        out.target = hdr.jobid_target();
        out.hasTarget = true;
    }
    out.eresult = hdr.has_eresult() ? hdr.eresult() : -1;
    return true;
}

void TrackServiceJob(std::uint64_t id, std::string name) {
    std::lock_guard<std::mutex> lk(s_jobNameMutex);
    if (s_jobNames.emplace(id, std::move(name)).second) {
        s_jobNameOrder.push_back(id);
    }
    while (s_jobNameOrder.size() > 512) {
        const std::uint64_t oldest = s_jobNameOrder.front();
        s_jobNameOrder.pop_front();
        s_jobNames.erase(oldest);
    }
}

std::string ResolveServiceJob(std::uint64_t jobIdTarget) {
    std::lock_guard<std::mutex> lk(s_jobNameMutex);
    auto it = s_jobNames.find(jobIdTarget);
    if (it == s_jobNames.end())
        return {};
    std::string out = std::move(it->second);
    s_jobNames.erase(it);
    const auto oit = std::find(s_jobNameOrder.begin(), s_jobNameOrder.end(), jobIdTarget);
    if (oit != s_jobNameOrder.end())
        s_jobNameOrder.erase(oit);
    return out;
}

} // namespace ac::hooks
