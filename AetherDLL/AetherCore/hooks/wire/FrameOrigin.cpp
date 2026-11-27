#include "pch.h"
#include "hooks/wire/FrameOrigin.h"

#include <mutex>

#include "core/Logger.h"
#include "hooks/wire/SendOriginPolicy.h"

#include "steam_messages.pb.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "PacketRouter";

std::mutex s_originMu;
std::uint32_t s_originConn = 0;
void* s_originObj = nullptr;
std::vector<std::uint8_t> s_originHeader;

std::uint32_t s_serviceConn = 0;
void* s_serviceObj = nullptr;
std::vector<std::uint8_t> s_serviceHeader;

bool ParseOriginHeader(std::uint32_t hConnection, void* obj, const WireFrame& f, CMsgProtoBufHeader& ph) {
    if (hConnection == 0 && !obj) {
        return false;
    }
    if (!f.header || f.headerLen == 0) {
        return false;
    }
    if (!ph.ParseFromArray(f.header, static_cast<int>(f.headerLen))) {
        return false;
    }
    return IsAuthenticatedCmOrigin(ph.steamid(), true);
}

} // namespace

void CaptureOrigin(std::uint32_t hConnection, void* obj, const WireFrame& f) {
    CMsgProtoBufHeader ph;
    if (!ParseOriginHeader(hConnection, obj, f, ph)) {
        return;
    }

    bool changed = false;
    {
        std::lock_guard lock(s_originMu);
        changed = (s_originHeader.empty() || s_originConn != hConnection || s_originObj != obj);
        s_originConn = hConnection;
        s_originObj = obj;
        s_originHeader.assign(f.header, f.header + f.headerLen);
    }
    if (changed) {
        AC_LOG_INFO(kModule, "Captured authenticated CM origin: hConn=0x%X ws=%p steamid=%llu sess=%d", hConnection,
                    obj, static_cast<unsigned long long>(ph.steamid()), ph.client_sessionid());
    }
}

void CaptureServiceOrigin(std::uint32_t hConnection, void* obj, const WireFrame& f) {
    if (f.eMsg != ac::constants::emsg::kServiceMethodCallFromClient &&
        f.eMsg != ac::constants::emsg::kServiceMethodCallFromClientAlt) {
        return;
    }
    CMsgProtoBufHeader ph;
    if (!ParseOriginHeader(hConnection, obj, f, ph)) {
        return;
    }
    if (!IsServiceCallOrigin(f.eMsg, ph.steamid(), true)) {
        return;
    }

    bool changed = false;
    {
        std::lock_guard lock(s_originMu);
        changed = (s_serviceHeader.empty() || s_serviceConn != hConnection || s_serviceObj != obj);
        s_serviceConn = hConnection;
        s_serviceObj = obj;
        s_serviceHeader.assign(f.header, f.header + f.headerLen);
    }
    if (changed) {
        AC_LOG_INFO(kModule, "Captured authenticated service origin: hConn=0x%X ws=%p steamid=%llu sess=%d",
                    hConnection, obj, static_cast<unsigned long long>(ph.steamid()), ph.client_sessionid());
    }
}

FrameOrigin SnapshotOrigin() {
    std::lock_guard lock(s_originMu);
    return FrameOrigin{s_originConn, s_originObj, s_originHeader};
}

FrameOrigin SnapshotServiceOrigin() {
    std::lock_guard lock(s_originMu);
    return FrameOrigin{s_serviceConn, s_serviceObj, s_serviceHeader};
}

} // namespace ac::hooks
