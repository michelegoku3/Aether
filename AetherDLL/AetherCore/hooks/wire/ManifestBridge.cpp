#include "pch.h"
#include "hooks/wire/ManifestBridge.h"

#include <optional>
#include <string>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"
#include "network/ManifestFetch.h"

#include "steam_messages.pb.h"

namespace ac::hooks::ManifestBridge {
namespace {

constexpr const char* kModule = "Wire.Manifest";
constexpr std::int32_t kNoChange = -1;

}  // namespace

std::int32_t HandleSend(const WireFrame& frame) {
    if (g_state.settings.manifestFetchUrls.empty()) return kNoChange;

    CContentServerDirectory_GetManifestRequestCode_Request req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) return kNoChange;
    if (!req.has_depot_id() || !req.has_manifest_id()) return kNoChange;

    const std::uint32_t depotId = req.depot_id();
    if (!luadata::HasDepot(depotId)) return kNoChange;  // real-owned: let it fly

    CMsgProtoBufHeader hdr;
    if (!hdr.ParseFromArray(frame.header, static_cast<int>(frame.headerLen)) ||
        !hdr.has_jobid_source()) {
        return kNoChange;
    }
    const std::uint64_t jobId = hdr.jobid_source();
    const std::uint64_t gid = req.manifest_id();
    const std::uint32_t appId = req.has_app_id() ? req.app_id() : 0;

    manifestfetch::Submit(jobId, gid, appId, depotId);
    AC_LOG_INFO(kModule, "Manifest lookup submitted: depot=%u gid=%llu job=%llu.", depotId,
                static_cast<unsigned long long>(gid), static_cast<unsigned long long>(jobId));
    return kNoChange;  // never rewrite the outgoing frame
}

std::int32_t HandleRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                        std::uint8_t* outHeader, std::uint32_t outHeaderCap,
                        std::int32_t* outHeaderLen) {
    if (!outHeaderLen) return kNoChange;
    CMsgProtoBufHeader hdr;
    if (!hdr.ParseFromArray(frame.header, static_cast<int>(frame.headerLen)) ||
        !hdr.has_jobid_target()) {
        return kNoChange;
    }
    const std::uint64_t jobId = hdr.jobid_target();

    std::optional<std::uint64_t> code = manifestfetch::Resolve(jobId);
    if (!code) return kNoChange;  // fetch failed; let Steam's original reply stand

    // Rewrite header eresult -> OK.
    hdr.set_eresult(static_cast<std::int32_t>(constants::kEResultOk));
    const std::uint32_t hdrSize = static_cast<std::uint32_t>(hdr.ByteSizeLong());
    if (hdrSize > outHeaderCap || !hdr.SerializeToArray(outHeader, static_cast<int>(outHeaderCap))) {
        return kNoChange;
    }
    *outHeaderLen = static_cast<std::int32_t>(hdrSize);

    // Body carries the fetched code.
    CContentServerDirectory_GetManifestRequestCode_Response resp;
    resp.set_manifest_request_code(*code);
    const std::uint32_t bodySize = static_cast<std::uint32_t>(resp.ByteSizeLong());
    if (bodySize > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
        *outHeaderLen = kNoChange;  // abort the header edit too
        return kNoChange;
    }
    AC_LOG_INFO(kModule, "Injected manifest code for job %llu.",
                static_cast<unsigned long long>(jobId));
    return static_cast<std::int32_t>(bodySize);
}

}  // namespace ac::hooks::ManifestBridge
