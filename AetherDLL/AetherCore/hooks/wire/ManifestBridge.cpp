#include "pch.h"
#include "hooks/wire/ManifestBridge.h"

#include <mutex>
#include <optional>

#include "core/Constants.h"
#include "core/Logger.h"
#include "hooks/wire/ManifestBridgePolicy.h"
#include "network/ManifestFetch.h"
#include "scripting/LuaData.h"

#include "steam_messages.pb.h"

namespace ac::hooks::ManifestBridge {
namespace {

constexpr const char* kModule = "Wire.Manifest";
constexpr std::int32_t kNoChange = -1;

std::mutex s_mutex;
manifestbridge::RequestTable s_requests;

} // namespace

std::int32_t HandleSend(const WireFrame& frame) {
    CContentServerDirectory_GetManifestRequestCode_Request req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen)))
        return kNoChange;
    if (!req.has_depot_id() || !req.has_manifest_id())
        return kNoChange;

    // Only depots a .lua configures: a genuinely owned depot keeps Steam's own
    // answer, which is what makes real licenses work.
    const std::uint32_t depotId = req.depot_id();
    if (!luadata::HasDepot(depotId))
        return kNoChange;

    CMsgProtoBufHeader hdr;
    if (!hdr.ParseFromArray(frame.header, static_cast<int>(frame.headerLen)) || !hdr.has_jobid_source())
        return kNoChange;

    const manifestbridge::PendingRequest request{req.has_app_id() ? req.app_id() : 0u, depotId, req.manifest_id()};
    {
        std::lock_guard<std::mutex> lock(s_mutex);
        s_requests.Record(hdr.jobid_source(), request);
    }
    manifestfetch::EnsureManifestAvailable(request.appId, request.depotId, request.gid);
    AC_LOG_INFO(kModule, "Manifest request code: app=%u depot=%u gid=%llu job=%llu.", request.appId, request.depotId,
                static_cast<unsigned long long>(request.gid), static_cast<unsigned long long>(hdr.jobid_source()));
    return kNoChange; // the request itself is never rewritten
}

std::int32_t HandleRecv(const WireFrame& frame, std::uint64_t jobIdTarget, std::uint8_t* out, std::uint32_t outCap,
                        std::uint8_t* outHeader, std::uint32_t outHeaderCap, std::int32_t* outHeaderLen) {
    if (!outHeaderLen)
        return kNoChange;

    std::optional<manifestbridge::PendingRequest> pending;
    {
        std::lock_guard<std::mutex> lock(s_mutex);
        pending = s_requests.Take(jobIdTarget);
    }
    if (!pending)
        return kNoChange;

    // No wait: this runs on Steam's network thread. A miss leaves Steam's own
    // reply in place; the background fetch started at send time finishes and
    // Steam's retry gets the code.
    const bool local = manifestfetch::HasLocalManifest(pending->gid, pending->depotId);
    if (manifestbridge::Decide(true, local) != manifestbridge::BridgeAction::Satisfy)
        return kNoChange;
    CMsgProtoBufHeader hdr;
    if (!hdr.ParseFromArray(frame.header, static_cast<int>(frame.headerLen)))
        return kNoChange;
    hdr.set_eresult(static_cast<std::int32_t>(constants::kEResultOk));
    const std::uint32_t headerSize = static_cast<std::uint32_t>(hdr.ByteSizeLong());
    if (headerSize > outHeaderCap || !hdr.SerializeToArray(outHeader, static_cast<int>(outHeaderCap)))
        return kNoChange;
    *outHeaderLen = static_cast<std::int32_t>(headerSize);

    CContentServerDirectory_GetManifestRequestCode_Response resp;
    resp.set_manifest_request_code(0);
    const std::uint32_t bodySize = static_cast<std::uint32_t>(resp.ByteSizeLong());
    if (bodySize > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
        *outHeaderLen = kNoChange; // abort the header edit too
        return kNoChange;
    }
    AC_LOG_INFO(kModule, "Manifest request code 0 for job %llu: depot=%u gid=%llu is local.",
                static_cast<unsigned long long>(jobIdTarget), pending->depotId,
                static_cast<unsigned long long>(pending->gid));
    return static_cast<std::int32_t>(bodySize);
}

} // namespace ac::hooks::ManifestBridge
