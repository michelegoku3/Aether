#include "pch.h"
#include "hooks/wire/PacketRouter.h"

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstring>
#include <deque>
#include <mutex>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "core/NetPacketAbi.h"
#include "hooks/wire/AccessTokenModule.h"
#include "hooks/wire/AchievementModule.h"
#include "hooks/wire/EticketModule.h"
#include "hooks/wire/FamilySharingModule.h"
#include "hooks/wire/FrameOrigin.h"
#include "hooks/wire/GamesPlayedModule.h"
#include "hooks/wire/PersonaInject.h"
#include "hooks/wire/ServiceJobTracker.h"
#include "hooks/wire/WireDiag.h"

#include "steam_messages.pb.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "Wire";
using namespace ac::constants;
using namespace ac::steam;

constexpr std::int32_t kNoChange = -1;

struct Pool {
    std::array<std::vector<std::uint8_t>, kWirePoolSlots> slots;
    std::uint32_t next = 0;
};

std::uint8_t* PoolSlot(std::uint32_t size) {
    // Packet hooks can run concurrently on different Steam threads. Keep the
    // temporary frame ring per-thread so slot selection and vector resize never
    // touch shared mutable state, while preserving buffer lifetime for nested
    // calls on the same thread. Callers must cap wire-controlled sizes (see
    // kWireMaxBodyBytes/kWireMaxHeaderBytes) before asking: an oversized
    // resize here would be retained per-thread for the whole Steam session.
    // Returns nullptr when the request exceeds the pool budget; the caller
    // then passes the frame through untouched (fail-open, never corrupt).
    constexpr std::uint32_t kPoolBudget = sizeof(MsgHdr) + kWireMaxHeaderBytes + kWireMaxBodyBytes;
    if (size > kPoolBudget)
        return nullptr;
    thread_local Pool t_pool;
    auto& buf = t_pool.slots[t_pool.next++ % kWirePoolSlots];
    buf.resize(size);
    return buf.data();
}

thread_local std::vector<std::uint8_t> t_scratchBody;
thread_local std::vector<std::uint8_t> t_scratchHeader;

void EnsureScratch() {
    if (t_scratchBody.size() < kWireMaxBodyBytes)
        t_scratchBody.resize(kWireMaxBodyBytes);
    if (t_scratchHeader.size() < kWireMaxHeaderBytes)
        t_scratchHeader.resize(kWireMaxHeaderBytes);
}

bool DecodeFrame(const std::uint8_t* data, std::uint32_t len, WireFrame& out) {
    if (!data || len < sizeof(MsgHdr)) {
        AC_LOG_TRACE_ONCE(kModule, "DecodeFrame failed: buffer null or size %u below MsgHdr size.", len);
        return false;
    }
    // memcpy-out, not reinterpret_cast dereference: Steam owns this buffer,
    // so alignment and strict-aliasing cannot be assumed. Same bytes out.
    MsgHdr hdr{};
    std::memcpy(&hdr, data, sizeof(hdr));
    // Skip non-protobuf packets — the headerLength field overlaps with payload
    // data and contains garbage values. Matches LumaCore's ParsePacket guard.
    if (!(hdr.eMsg & kMsgHdrProtoFlag)) {
        return false;
    }
    const std::uint32_t eMsg = hdr.eMsg & ~kMsgHdrProtoFlag;
    const std::uint32_t headerLen = hdr.headerLength;
    if (sizeof(MsgHdr) + headerLen > len) {
        AC_LOG_TRACE_ONCE(kModule, "DecodeFrame failed: header length %u exceeds total frame length %u.", headerLen,
                          len);
        return false;
    }
    out.eMsg = eMsg;
    out.header = data + sizeof(MsgHdr);
    out.headerLen = headerLen;
    out.body = data + sizeof(MsgHdr) + headerLen;
    out.bodyLen = len - sizeof(MsgHdr) - headerLen;
    return true;
}

// Thin dispatch core: frame decode + pool live here, feature routing
// delegates to focused modules. Service-job correlation lives in
// ServiceJobTracker, diagnostics in WireDiag, connection capture in
// FrameOrigin (same behavior, split TU for reviewability).

std::int32_t DispatchSend(const WireFrame& f) {
    EnsureScratch();
    TraceFrame("send", f);
    switch (f.eMsg) {
    case emsg::kClientPICSProductInfoRequest:
        return AccessToken::HandleSend(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kClientGamesPlayed:
    case emsg::kClientGamesPlayedWithDataBlob:
        return GamesPlayed::HandleSend(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kClientRichPresenceUpload:
        return GamesPlayed::HandleRichPresenceUpload(f);
    case emsg::kClientGetUserStats:
        return AchievementModule::HandleSendClientGetUserStats(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kClientStoreUserStats2:
        return AchievementModule::HandleSendStoreUserStats2(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kServiceMethodCallFromClient:
    // Newer Steam builds (observed on build 1788652215) send the same
    // service calls with eMsg 146: without this case the stats
    // handlers never see them.
    case emsg::kServiceMethodCallFromClientAlt: {
        ServiceRequest req;
        if (ParseServiceRequest(f, req) && req.hasName) {
            if (req.hasSource)
                TrackServiceJob(req.source, req.name); // [DIAG] flight-recorder jobid->name
            std::uint32_t h = FnvHash(req.name.c_str());
            if (h == job_hash::kGetUserStats) {
                return AchievementModule::HandleSendGetUserStats(f, t_scratchBody.data(), kWireMaxBodyBytes);
            }
        }
        return kNoChange;
    }
    default:
        return kNoChange;
    }
}

using BBuildAndAsyncSendFrame_t = bool (*)(void*, steam::EWebSocketOpCode, std::uint8_t*, std::uint32_t);
using RecvPkt_t = void* (*)(void*, steam::CNetPacket*);

BBuildAndAsyncSendFrame_t o_BBuildAndAsyncSendFrame = nullptr;
RecvPkt_t o_RecvPkt = nullptr;

bool h_BBuildAndAsyncSendFrame(void* obj, steam::EWebSocketOpCode opcode, std::uint8_t* data, std::uint32_t len) {
    if (opcode != steam::k_eWebSocketOpCode_Binary) {
        return o_BBuildAndAsyncSendFrame(obj, opcode, data, len);
    }

    WireFrame f;
    if (!DecodeFrame(data, len, f)) {
        if (data && len >= sizeof(MsgHdr)) {
            MsgHdr hdr{};
            std::memcpy(&hdr, data, sizeof(hdr)); // foreign buffer: see DecodeFrame
            const std::uint32_t raw = hdr.eMsg;
            AC_LOG_TRACE(kModule, "send undecoded eMsg=%u proto=%d len=%u", raw & ~kMsgHdrProtoFlag,
                         (raw & kMsgHdrProtoFlag) ? 1 : 0, len);
        }
    } else {
        CaptureOrigin(obj, f);
        std::int32_t newBodyLen = DispatchSend(f);
        // Defense in depth: handlers must report a length within the scratch
        // budget they were given. A misbehaving handler must degrade to
        // passthrough, never to an oversized pool retention or over-read.
        if (newBodyLen < 0 || static_cast<std::uint32_t>(newBodyLen) > kWireMaxBodyBytes) {
            if (newBodyLen > 0)
                AC_LOG_WARN_ONCE(kModule, "Handler over-reported body length (%d); passing frame through.", newBodyLen);
        } else {
            const std::uint32_t newSize = sizeof(MsgHdr) + f.headerLen + static_cast<std::uint32_t>(newBodyLen);
            std::uint8_t* buf = PoolSlot(newSize);
            if (!buf)
                return o_BBuildAndAsyncSendFrame(obj, opcode, data, len);
            std::memcpy(buf, data, sizeof(MsgHdr));
            reinterpret_cast<MsgHdr*>(buf)->headerLength = f.headerLen;
            std::memcpy(buf + sizeof(MsgHdr), f.header, f.headerLen);
            std::memcpy(buf + sizeof(MsgHdr) + f.headerLen, t_scratchBody.data(), newBodyLen);
            return o_BBuildAndAsyncSendFrame(obj, opcode, buf, newSize);
        }
    }
    return o_BBuildAndAsyncSendFrame(obj, opcode, data, len);
}

thread_local std::int32_t t_recvHeaderLen = kNoChange;

std::int32_t DispatchRecv(const WireFrame& f) {
    EnsureScratch();
    t_recvHeaderLen = kNoChange;
    TraceFrame("recv", f);
    switch (f.eMsg) {
    case emsg::kServiceMethodResponse: {
        // [DIAG] flight-recorder: the service name is rebuilt via the
        // jobid tracked at send time. ResolveServiceJob must always run
        // (it consumes the entry and keeps s_jobNames bounded), but the
        // TRACE and the diag ring are reserved for NON-OK responses:
        // those are what StatusWriter aggregates (AccessDenied /
        // transport-candidate), and logging every OK would cost a mutex
        // + allocations per frame on the network path.
        ServiceResponse rsp;
        const bool rspParsed = ParseServiceResponse(f, rsp);
        {
            // Unparsable headers keep the historical outcome: eresult -1
            // (missing_or_unparsed), no job resolution, no dispatch below.
            const std::uint64_t jt = (rspParsed && rsp.hasTarget) ? rsp.target : 0;
            const std::int32_t er = rspParsed ? rsp.eresult : -1;
            const std::string jname = jt ? ResolveServiceJob(jt) : std::string();
            if (er != 1) { // 1 = k_EResultOK
                AC_LOG_TRACE(kModule, "[DIAG] service recv name='%s' jobid=%llu eresult=%d (%s) class=%s bLen=%u",
                             jname.empty() ? "?" : jname.c_str(), static_cast<unsigned long long>(jt), er,
                             EresultName(er), EresultClass(er), f.bodyLen);
                diag::Record("wire_eresult", "service=" + (jname.empty() ? std::string("?") : jname) +
                                                 " jobid=" + std::to_string(jt) + " eresult=" + std::to_string(er) +
                                                 " label=" + EresultName(er) + " class=" + EresultClass(er));
            }
        }
        if (!rspParsed || !rsp.hasName)
            return kNoChange;
        std::uint32_t h = FnvHash(rsp.name.c_str());
        if (h == job_hash::kNotifyRunningApps) {
            return FamilySharing::ShouldSuppress() ? FamilySharing::ClearBody() : kNoChange;
        }
        // Request-code responses pass through unchanged: acquisition is
        // local/Hubcap (the bridge was removed).
        if (h == job_hash::kGetUserStats) {
            return AchievementModule::HandleRecvGetUserStatsResponse(f, t_scratchBody.data(), kWireMaxBodyBytes,
                                                                     t_scratchHeader.data(), kWireMaxHeaderBytes,
                                                                     &t_recvHeaderLen);
        }
        return kNoChange;
    }
    case emsg::kClientGetUserStatsResponse:
        return AchievementModule::HandleRecvClientGetUserStatsResponse(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kClientPersonaState:
        return PersonaInject::OnPersonaStateRecv(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kClientRequestEncryptedAppTicketResponse:
        return EticketModule::HandleRecv(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kClientSharedLibraryLockStatus:
    case emsg::kClientSharedLibraryStopPlaying:
        // [DIAG] These are the messages with which the CM orders the
        // client to stop playing: if a masked (Spacewar) session dies,
        // this line tells whether the server ordered it or the local
        // client decided on its own.
        AC_LOG_INFO(kModule, "[DIAG] SharedLibrary msg eMsg=%u bodyLen=%u suppress=%d", f.eMsg, f.bodyLen,
                    FamilySharing::ShouldSuppress() ? 1 : 0);
        return FamilySharing::ShouldSuppress() ? FamilySharing::ClearBody() : kNoChange;
    default:
        return kNoChange;
    }
}

void* h_RecvPkt(void* self, CNetPacket* packet) {
    // GATE: nothing below may read or write a packet field until the
    // per-build layout has been identified from live traffic. Before
    // that (and forever, if the probe gives up) the packet travels
    // through untouched — Steam boots, we simply do less.
    if (packet && abi::netpkt::EnsureResolved(packet)) {
        std::uint8_t*& pktData = abi::netpkt::Data(packet);
        std::uint32_t& pktLen = abi::netpkt::Size(packet);
        // Write barrier: re-checks the latched layout against THIS
        // object before we are allowed to mutate it. Reading is
        // already safe (EnsureResolved + DecodeFrame validate the
        // frame); writing is what corrupts Steam, so it gets its own
        // gate.
        const bool mayWrite = abi::netpkt::BeginWrite(packet);

        WireFrame f;
        if (DecodeFrame(pktData, pktLen, f)) {
            if (f.eMsg == emsg::kMulti) {
                // [DIAG] flight-recorder: Multi frames never enter the
                // dispatch, but they must stay visible in the trace (with
                // the signature of the first body bytes, to recognize
                // a compressed payload's zlib header if decoding is needed).
                static constexpr char kHex[] = "0123456789ABCDEF";
                char head[8 * 2 + 1] = {};
                const std::uint32_t n = f.bodyLen < 8 ? f.bodyLen : 8;
                for (std::uint32_t i = 0; i < n; ++i) {
                    const std::size_t o = static_cast<std::size_t>(i) * 2;
                    head[o] = kHex[(f.body[i] >> 4) & 0xF];
                    head[o + 1] = kHex[f.body[i] & 0xF];
                }
                AC_LOG_TRACE(kModule, "recv eMsg=1 (Multi) bLen=%u head8=%s", f.bodyLen, head);
            } else {
                const std::int32_t newBodyLen = DispatchRecv(f);

                if (!mayWrite) {
                    // Dispatch already ran (handlers observe traffic and
                    // keep their caches warm); only the rewrite is
                    // dropped.
                    AC_LOG_TRACE_ONCE(kModule, "Rewrite skipped: packet rejected by the ABI write barrier.");
                } else if (t_recvHeaderLen >= 0 && newBodyLen >= 0) {
                    const std::uint32_t newSize = sizeof(MsgHdr) + t_recvHeaderLen + newBodyLen;
                    std::uint8_t* buf = PoolSlot(newSize);
                    if (buf) {
                        std::memcpy(buf, pktData, sizeof(MsgHdr));
                        reinterpret_cast<MsgHdr*>(buf)->headerLength = t_recvHeaderLen;
                        std::memcpy(buf + sizeof(MsgHdr), t_scratchHeader.data(), t_recvHeaderLen);
                        std::memcpy(buf + sizeof(MsgHdr) + t_recvHeaderLen, t_scratchBody.data(), newBodyLen);
                        pktData = buf;
                        pktLen = newSize;
                    }
                } else if (newBodyLen == 0) {
                    pktLen = sizeof(MsgHdr) + f.headerLen;
                } else if (newBodyLen > 0) {
                    const std::uint32_t newSize = sizeof(MsgHdr) + f.headerLen + newBodyLen;
                    std::uint8_t* buf = PoolSlot(newSize);
                    if (buf) {
                        std::memcpy(buf, pktData, sizeof(MsgHdr));
                        reinterpret_cast<MsgHdr*>(buf)->headerLength = f.headerLen;
                        std::memcpy(buf + sizeof(MsgHdr), f.header, f.headerLen);
                        std::memcpy(buf + sizeof(MsgHdr) + f.headerLen, t_scratchBody.data(), newBodyLen);
                        pktData = buf;
                        pktLen = newSize;
                    }
                }
            }
        }

        PersonaInject::TryDeliver(self, packet, o_RecvPkt);
    }
    return o_RecvPkt(self, packet);
}

} // namespace

bool SendClientFrame(std::uint32_t eMsg, const std::uint8_t* body, std::uint32_t bodyLen) {
    if (!o_BBuildAndAsyncSendFrame) {
        AC_LOG_WARN(kModule, "SendClientFrame: send hook not installed.");
        return false;
    }
    if (bodyLen > kWireMaxBodyBytes)
        return false;

    const FrameOrigin origin = SnapshotOrigin();
    void* obj = origin.obj;
    std::vector<std::uint8_t> hdrBytes = origin.header;
    if (!obj || hdrBytes.empty()) {
        AC_LOG_WARN(kModule, "SendClientFrame: no captured connection yet.");
        return false;
    }

    // Re-serialize the captured header with fresh job ids: keep steamid and
    // client_sessionid, drop any correlation belonging to the borrowed message.
    CMsgProtoBufHeader ph;
    if (!ph.ParseFromArray(hdrBytes.data(), static_cast<int>(hdrBytes.size()))) {
        AC_LOG_WARN(kModule, "SendClientFrame: captured header unparsable.");
        return false;
    }
    ph.clear_jobid_source();
    ph.clear_jobid_target();
    ph.clear_target_job_name();
    ph.clear_eresult();

    std::string hdrOut;
    if (!ph.SerializeToString(&hdrOut) || hdrOut.size() > kWireMaxHeaderBytes)
        return false;

    const std::uint32_t total = sizeof(MsgHdr) + static_cast<std::uint32_t>(hdrOut.size()) + bodyLen;
    std::vector<std::uint8_t> frame(total);
    auto* mh = reinterpret_cast<MsgHdr*>(frame.data());
    mh->eMsg = eMsg | kMsgHdrProtoFlag;
    mh->headerLength = static_cast<std::uint32_t>(hdrOut.size());
    std::memcpy(frame.data() + sizeof(MsgHdr), hdrOut.data(), hdrOut.size());
    if (bodyLen > 0) {
        std::memcpy(frame.data() + sizeof(MsgHdr) + hdrOut.size(), body, bodyLen);
    }

    const bool ok = o_BBuildAndAsyncSendFrame(obj, steam::k_eWebSocketOpCode_Binary, frame.data(), total);
    AC_LOG_INFO(kModule, "[DIAG] SendClientFrame eMsg=%u hdr=%zu body=%u steamid=%llu sess=%d -> %s", eMsg,
                hdrOut.size(), bodyLen, static_cast<unsigned long long>(ph.steamid()), ph.client_sessionid(),
                ok ? "ok" : "FAILED");
    return ok;
}

void RegisterPacketRouter(HMODULE diversion) {
    const auto settings = Settings::Snapshot();
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering packet router.");

    // Build/config signature: Settings::Load runs BEFORE the logger is
    // up, so this stamp is the only reliable proof of WHICH build is
    // running. If it never appears in the log, the loaded DLL is not the
    // patched build (the updater reports the same version for both).
    AC_LOG_INFO(kModule,
                "[DIAG] BUILD showonline-suffix+fix14 | inject_local=%d always_extra_info=%d "
                "showonline_broadcast=%d friend_appid_from_name=%d appid_blob=%d suffix_invisible=%d",
                settings->presenceInjectLocal ? 1 : 0, settings->presenceAlwaysExtraInfo ? 1 : 0,
                settings->presenceShowOnlineBroadcast ? 1 : 0, settings->presenceFriendAppIdFromName ? 1 : 0,
                settings->presenceAppIdBlob ? 1 : 0, settings->presenceSuffixInvisible ? 1 : 0);

    g_state.hookManager.TryHook("BBuildAndAsyncSendFrame", "steamclient", diversion, o_BBuildAndAsyncSendFrame,
                                h_BBuildAndAsyncSendFrame);
    g_state.hookManager.TryHook("RecvPkt", "steamclient", diversion, o_RecvPkt, h_RecvPkt);
}

} // namespace ac::hooks
