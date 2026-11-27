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
#include "hooks/license/LicenseManager.h"
#include "hooks/wire/AccessTokenModule.h"
#include "hooks/wire/AchievementModule.h"
#include "hooks/wire/AchievementUi.h"
#include "hooks/wire/EticketModule.h"
#include "hooks/wire/FrameOrigin.h"
#include "hooks/wire/GamesPlayedModule.h"
#include "hooks/wire/ManifestBridge.h"
#include "hooks/wire/PersonaInject.h"
#include "hooks/wire/SendOriginPolicy.h"
#include "hooks/wire/ServiceJobTracker.h"
#include "hooks/wire/WireDiag.h"
#include "utils/PatternEngine.h"

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
    // data and contains garbage values.
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
// FrameOrigin.

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
            if (h == job_hash::kGetManifestRequestCode)
                return ManifestBridge::HandleSend(f);
            if (h == job_hash::kGetUserStats) {
                return AchievementModule::HandleSendGetUserStats(f, t_scratchBody.data(), kWireMaxBodyBytes);
            }
            // Client achievement page: remember the job id so the reply can be
            // attributed, and start the local read before the reply lands. The
            // requests themselves leave untouched.
            if (h == job_hash::kGetGameAchievements) {
                if (req.hasSource)
                    ac::achieve::OnSendSchema(req.source, f.body, f.bodyLen);
                return kNoChange;
            }
            if (h == job_hash::kGetUserAchievements) {
                if (req.hasSource)
                    ac::achieve::OnSendUser(req.source, f.body, f.bodyLen);
                return kNoChange;
            }
        }
        return kNoChange;
    }
    default:
        return kNoChange;
    }
}

using CNetBAsyncSend_t = bool (*)(std::uint32_t, std::uint32_t, const std::uint8_t*, std::uint32_t);
using CNetBAsyncSendSwapReserved_t = bool (*)(std::uint32_t, std::uint32_t, steam::CUtlBuffer*, std::int32_t);
using BBuildAndAsyncSendFrame_t = bool (*)(void*, steam::EWebSocketOpCode, std::uint8_t*, std::uint32_t);
using RecvPkt_t = void* (*)(void*, steam::CNetPacket*);

CNetBAsyncSend_t o_CNetBAsyncSend = nullptr;
CNetBAsyncSendSwapReserved_t o_CNetBAsyncSendSwapReserved = nullptr;
BBuildAndAsyncSendFrame_t o_BBuildAndAsyncSendFrame = nullptr;
RecvPkt_t o_RecvPkt = nullptr;

// Fallback byte signatures when per-build pattern tables omit the CNet send functions.
constexpr const char* kSigCNetBAsyncSend =
    "40 53 55 56 57 48 83 EC 48 48 8B 05 ?? ?? ?? ?? 41 8B F9 49 8B F0 8B DA 8B E9 44 8B 10 45 85 D2 75 04 33 C0 EB "
    "17 4C 8D 44 24 30 BA 06 00 00 00 48 8D 0D ?? ?? ?? ?? FF 15 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 89 44 24 30 "
    "48 85 C9 74 08 FF 41 68 E8 ?? ?? ?? ?? 8B CB E8 ?? ?? ?? ?? 48 8B D8 48 85 C0 75 1B ?? ?? ?? ?? ?? ?? ?? ?? "
    "?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? E8 ?? ?? ?? ?? 44 8B C7 8B D5 48 8B C8 E8 ?? ?? ?? "
    "?? 45 33 C9";

constexpr const char* kSigCNetBAsyncSendSwapReserved =
    "40 53 55 56 57 48 83 EC 48 48 8B 05 ?? ?? ?? ?? 41 8B F9 49 8B F0 8B DA 8B E9 44 8B 10 45 85 D2 75 04 33 C0 EB "
    "17 4C 8D 44 24 30 BA 06 00 00 00 48 8D 0D ?? ?? ?? ?? FF 15 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 89 44 24 30 "
    "48 85 C9 74 08 FF 41 68 E8 ?? ?? ?? ?? 8B CB E8 ?? ?? ?? ?? 48 8B D8 48 85 C0 75 1B ?? ?? ?? ?? ?? ?? ?? ?? "
    "?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 39 7E 14 7D 28";

thread_local bool t_inCNetSend = false;

struct ScopedCNetSendGuard {
    bool& flag;
    bool prev;
    explicit ScopedCNetSendGuard(bool& f) : flag(f), prev(f) { flag = true; }
    ~ScopedCNetSendGuard() { flag = prev; }
};

std::once_flag s_cnetSendSeenOnce;
std::once_flag s_cnetSwapSeenOnce;
std::once_flag s_wsFallbackSeenOnce;

// Decodes one outbound CM frame, captures authenticated transport identity, and
// builds a rewritten frame in the per-thread pool when a handler modifies the body.
bool PrepareOutboundRewrite(std::uint32_t hConnection, void* wsObj, const std::uint8_t* data, std::uint32_t len,
                            std::uint8_t*& outBuf, std::uint32_t& outLen) {
    outBuf = nullptr;
    outLen = 0;

    WireFrame f;
    if (!DecodeFrame(data, len, f)) {
        if (data && len >= sizeof(MsgHdr)) {
            MsgHdr hdr{};
            std::memcpy(&hdr, data, sizeof(hdr)); // foreign buffer: see DecodeFrame
            const std::uint32_t raw = hdr.eMsg;
            AC_LOG_TRACE(kModule, "send undecoded eMsg=%u proto=%d len=%u", raw & ~kMsgHdrProtoFlag,
                         (raw & kMsgHdrProtoFlag) ? 1 : 0, len);
        }
        return false;
    }

    CaptureOrigin(hConnection, wsObj, f);
    CaptureServiceOrigin(hConnection, wsObj, f);

    const std::int32_t newBodyLen = DispatchSend(f);
    // Defense in depth: handlers must report a length within the scratch
    // budget they were given. A misbehaving handler degrades to passthrough.
    if (newBodyLen < 0 || static_cast<std::uint32_t>(newBodyLen) > kWireMaxBodyBytes) {
        if (newBodyLen > 0) {
            AC_LOG_WARN_ONCE(kModule, "Handler over-reported body length (%d); passing frame through.", newBodyLen);
        }
        return false;
    }

    const std::uint32_t newSize = sizeof(MsgHdr) + f.headerLen + static_cast<std::uint32_t>(newBodyLen);
    std::uint8_t* buf = PoolSlot(newSize);
    if (!buf) {
        return false;
    }
    std::memcpy(buf, data, sizeof(MsgHdr));
    reinterpret_cast<MsgHdr*>(buf)->headerLength = f.headerLen;
    std::memcpy(buf + sizeof(MsgHdr), f.header, f.headerLen);
    if (newBodyLen > 0) {
        std::memcpy(buf + sizeof(MsgHdr) + f.headerLen, t_scratchBody.data(), static_cast<std::size_t>(newBodyLen));
    }
    outBuf = buf;
    outLen = newSize;
    return true;
}

bool h_CNetBAsyncSend(std::uint32_t eMsgHint, std::uint32_t hConnection, const std::uint8_t* data, std::uint32_t len) {
    if (t_inCNetSend || ShouldSkipRawSendHint(eMsgHint)) {
        return o_CNetBAsyncSend(eMsgHint, hConnection, data, len);
    }

    std::call_once(s_cnetSendSeenOnce, [&]() {
        AC_LOG_INFO(kModule, "[DIAG] CNet::BAsyncSend live interception active (hint=%u hConn=0x%X len=%u)", eMsgHint,
                    hConnection, len);
    });

    ScopedCNetSendGuard guard(t_inCNetSend);
    std::uint8_t* rewrittenBuf = nullptr;
    std::uint32_t rewrittenLen = 0;
    if (PrepareOutboundRewrite(hConnection, nullptr, data, len, rewrittenBuf, rewrittenLen)) {
        return o_CNetBAsyncSend(eMsgHint, hConnection, rewrittenBuf, rewrittenLen);
    }
    return o_CNetBAsyncSend(eMsgHint, hConnection, data, len);
}

bool h_CNetBAsyncSendSwapReserved(std::uint32_t eMsgHint, std::uint32_t hConnection, steam::CUtlBuffer* buf,
                                  std::int32_t cubReserved) {
    if (t_inCNetSend || !buf) {
        return o_CNetBAsyncSendSwapReserved(eMsgHint, hConnection, buf, cubReserved);
    }

    const std::uint8_t* sliceData = nullptr;
    std::uint32_t sliceLen = 0;
    if (!ExtractReservedBufferSlice(buf->Base(), buf->TellPut(), cubReserved, sliceData, sliceLen)) {
        return o_CNetBAsyncSendSwapReserved(eMsgHint, hConnection, buf, cubReserved);
    }

    std::call_once(s_cnetSwapSeenOnce, [&]() {
        AC_LOG_INFO(kModule, "[DIAG] CNet::BAsyncSendSwapReserved live interception active (hint=%u hConn=0x%X len=%u)",
                    eMsgHint, hConnection, sliceLen);
    });

    ScopedCNetSendGuard guard(t_inCNetSend);
    std::uint8_t* rewrittenBuf = nullptr;
    std::uint32_t rewrittenLen = 0;
    if (PrepareOutboundRewrite(hConnection, nullptr, sliceData, sliceLen, rewrittenBuf, rewrittenLen)) {
        if (o_CNetBAsyncSend) {
            // Send the pooled copy directly via CNet::BAsyncSend so Steam's
            // caller CUtlBuffer remains intact for its normal cleanup.
            return o_CNetBAsyncSend(eMsgHint, hConnection, rewrittenBuf, rewrittenLen);
        }
        AC_LOG_WARN_ONCE(kModule,
                         "CNet::BAsyncSendSwapReserved rewrote frame without o_CNetBAsyncSend; passing through.");
    }
    return o_CNetBAsyncSendSwapReserved(eMsgHint, hConnection, buf, cubReserved);
}

bool h_BBuildAndAsyncSendFrame(void* obj, steam::EWebSocketOpCode opcode, std::uint8_t* data, std::uint32_t len) {
    if (t_inCNetSend || opcode != steam::k_eWebSocketOpCode_Binary) {
        return o_BBuildAndAsyncSendFrame(obj, opcode, data, len);
    }

    std::call_once(s_wsFallbackSeenOnce,
                   [&]() { AC_LOG_INFO(kModule, "[DIAG] BBuildAndAsyncSendFrame WebSocket fallback active"); });

    std::uint8_t* rewrittenBuf = nullptr;
    std::uint32_t rewrittenLen = 0;
    if (PrepareOutboundRewrite(0, obj, data, len, rewrittenBuf, rewrittenLen)) {
        return o_BBuildAndAsyncSendFrame(obj, opcode, rewrittenBuf, rewrittenLen);
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
            // Unparsable headers yield eresult -1 (missing_or_unparsed), no job
            // resolution and no dispatch below.
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
        // Request-code replies: answer with code 0 only when this session saw
        // the request and the exact manifest is already on disk.
        if (h == job_hash::kGetManifestRequestCode) {
            return ManifestBridge::HandleRecv(f, rsp.hasTarget ? rsp.target : 0, t_scratchBody.data(),
                                              kWireMaxBodyBytes, t_scratchHeader.data(), kWireMaxHeaderBytes,
                                              &t_recvHeaderLen);
        }
        if (h == job_hash::kGetUserStats) {
            return AchievementModule::HandleRecvGetUserStatsResponse(f, t_scratchBody.data(), kWireMaxBodyBytes,
                                                                     t_scratchHeader.data(), kWireMaxHeaderBytes,
                                                                     &t_recvHeaderLen);
        }
        // Client achievement page: learn the schema from the reply the client
        // already receives, and rebuild the user-state reply from local data.
        // Neither handler touches the header.
        if (h == job_hash::kGetGameAchievements) {
            return ac::achieve::OnRecvSchema(rsp.hasTarget ? rsp.target : 0, f.body, f.bodyLen);
        }
        if (h == job_hash::kGetUserAchievements) {
            return ac::achieve::OnRecvUser(rsp.hasTarget ? rsp.target : 0, f.header, f.headerLen, f.body, f.bodyLen,
                                           t_scratchHeader.data(), kWireMaxHeaderBytes, &t_recvHeaderLen,
                                           t_scratchBody.data(), kWireMaxBodyBytes);
        }
        return kNoChange;
    }
    case emsg::kClientGetUserStatsResponse:
        return AchievementModule::HandleRecvClientGetUserStatsResponse(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kClientPersonaState:
        return PersonaInject::OnPersonaStateRecv(f, t_scratchBody.data(), kWireMaxBodyBytes);
    case emsg::kClientRequestEncryptedAppTicketResponse:
        return EticketModule::HandleRecv(f, t_scratchBody.data(), kWireMaxBodyBytes);
    default:
        return kNoChange;
    }
}

void* h_RecvPkt(void* self, CNetPacket* packet) {
    LicenseManager::RecordSteamMainThread("RecvPkt");
    // Replies the achievement page had to park are delivered on this carrier,
    // before its own frame is dispatched: a frame queued during a dispatch must
    // go out on the next packet, not replace the one that produced it.
    ac::achieve::TryDeliver(self, packet, o_RecvPkt);
    // GATE: nothing below may read or write a packet field until the
    // per-build layout has been identified from live traffic. Before
    // that (and forever, if the probe gives up) the packet travels
    // through untouched — Steam boots, we simply do less.
    if (packet && abi::netpkt::EnsureResolved(packet)) {
        std::uint8_t*& pktData = abi::netpkt::Data(packet);
        std::uint32_t& pktLen = abi::netpkt::Size(packet);

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

                // Only a handler that produced a new body needs the packet
                // mutated. The barrier re-checks the latched layout against
                // THIS object before we may write; running it here instead of
                // on every inbound frame keeps its syscalls off the common
                // pass-through path, where nothing is rewritten.
                if (newBodyLen != kNoChange) {
                    if (!abi::netpkt::BeginWrite(packet)) {
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
        }

        PersonaInject::TryDeliver(self, packet, o_RecvPkt);
    }
    void* result = o_RecvPkt(self, packet);
    LicenseManager::TryDrainOnSteamThread();
    return result;
}

} // namespace

namespace {

template <typename Fn>
bool TryHookWithFallback(const char* name, HMODULE steamclient, const char* fallbackSig, Fn& original, Fn detour,
                         const char*& sourceOut) {
    bool byScan = false;
    void* target = ac::pattern::ResolveOrScanAddress(name, "steamclient", steamclient, fallbackSig, &byScan);
    if (!target) {
        sourceOut = "missing";
        return false;
    }
    sourceOut = byScan ? "scan" : "toml";
    g_state.hookManager.RegisterHook(name, target, reinterpret_cast<void**>(&original),
                                     reinterpret_cast<void*>(detour));
    return true;
}

bool SendFrameImpl(std::uint32_t eMsg, const std::uint8_t* body, std::uint32_t bodyLen, const char* targetJobName,
                   std::uint64_t jobId) {
    if (!o_CNetBAsyncSend && !o_BBuildAndAsyncSendFrame) {
        AC_LOG_WARN(kModule, "SendClientFrame: send hook not installed.");
        return false;
    }
    if (bodyLen > kWireMaxBodyBytes)
        return false;

    // A service call must borrow the identity of a real service call: the CM
    // drops one composed from a heartbeat's header (steamid 0).
    const FrameOrigin origin = targetJobName ? SnapshotServiceOrigin() : SnapshotOrigin();
    const bool useCNet = (o_CNetBAsyncSend != nullptr && origin.hConnection != 0);
    const bool useWs = (!useCNet && o_BBuildAndAsyncSendFrame != nullptr && origin.obj != nullptr);
    const std::vector<std::uint8_t>& hdrBytes = origin.header;
    if ((!useCNet && !useWs) || hdrBytes.empty()) {
        AC_LOG_WARN(kModule, "SendClientFrame: no captured %s yet (hConn=0x%X ws=%p).",
                    targetJobName ? "service call" : "connection", origin.hConnection, origin.obj);
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
    if (targetJobName) {
        ph.set_target_job_name(targetJobName);
        ph.set_jobid_source(jobId);
    }

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

    const bool ok = useCNet
                        ? o_CNetBAsyncSend(eMsg, origin.hConnection, frame.data(), total)
                        : o_BBuildAndAsyncSendFrame(origin.obj, steam::k_eWebSocketOpCode_Binary, frame.data(), total);
    AC_LOG_INFO(kModule,
                "[DIAG] SendClientFrame via=%s eMsg=%u hConn=0x%X hdr=%zu body=%u steamid=%llu sess=%d job=%s -> %s",
                useCNet ? "cnet" : "ws", eMsg, origin.hConnection, hdrOut.size(), bodyLen,
                static_cast<unsigned long long>(ph.steamid()), ph.client_sessionid(),
                targetJobName ? targetJobName : "-", ok ? "ok" : "FAILED");
    return ok;
}

} // namespace

bool SendClientFrame(std::uint32_t eMsg, const std::uint8_t* body, std::uint32_t bodyLen) {
    return SendFrameImpl(eMsg, body, bodyLen, nullptr, 0);
}

bool SendClientServiceCall(const char* targetJobName, std::uint64_t jobId, const std::uint8_t* body,
                           std::uint32_t bodyLen) {
    if (!targetJobName || !*targetJobName)
        return false;
    return SendFrameImpl(emsg::kServiceMethodCallFromClient, body, bodyLen, targetJobName, jobId);
}

bool CanSendServiceCall() {
    const FrameOrigin origin = SnapshotServiceOrigin();
    if (origin.header.empty())
        return false;
    const bool useCNet = (o_CNetBAsyncSend != nullptr && origin.hConnection != 0);
    const bool useWs = (o_BBuildAndAsyncSendFrame != nullptr && origin.obj != nullptr);
    return useCNet || useWs;
}

void RegisterPacketRouter(HMODULE steamclient) {
    const auto settings = Settings::Snapshot();
    if (!steamclient) {
        AC_LOG_ERROR(kModule, "Steamclient module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering packet router.");

    // Build/config signature: Settings::Load runs BEFORE the logger is
    // up, so this stamp is the only reliable proof of WHICH build is
    // running. If it never appears in the log, the loaded DLL is not the
    // patched build (the updater reports the same version for both).
    AC_LOG_INFO(kModule,
                "[DIAG] BUILD unified-cnet-send | inject_local=%d always_extra_info=%d "
                "showonline_broadcast=%d friend_appid_from_name=%d appid_blob=%d suffix_invisible=%d",
                settings->presenceInjectLocal ? 1 : 0, settings->presenceAlwaysExtraInfo ? 1 : 0,
                settings->presenceShowOnlineBroadcast ? 1 : 0, settings->presenceFriendAppIdFromName ? 1 : 0,
                settings->presenceAppIdBlob ? 1 : 0, settings->presenceSuffixInvisible ? 1 : 0);

    const char* cnetSendSource = "missing";
    const char* cnetSwapSource = "missing";
    const bool cnetSendHooked = TryHookWithFallback("CNetBAsyncSend", steamclient, kSigCNetBAsyncSend, o_CNetBAsyncSend,
                                                    h_CNetBAsyncSend, cnetSendSource);
    const bool cnetSwapHooked =
        TryHookWithFallback("CNetBAsyncSendSwapReserved", steamclient, kSigCNetBAsyncSendSwapReserved,
                            o_CNetBAsyncSendSwapReserved, h_CNetBAsyncSendSwapReserved, cnetSwapSource);

    const bool fallbackWs = ShouldFallbackToWebSocket(cnetSendHooked, cnetSwapHooked);
    bool wsHooked = false;
    if (fallbackWs) {
        wsHooked = g_state.hookManager.TryHook("BBuildAndAsyncSendFrame", "steamclient", steamclient,
                                               o_BBuildAndAsyncSendFrame, h_BBuildAndAsyncSendFrame);
    }

    g_state.hookManager.TryHook("RecvPkt", "steamclient", steamclient, o_RecvPkt, h_RecvPkt);

    AC_LOG_INFO(kModule, "[DIAG] Wire send mode: %s (CNetBAsyncSend=%s CNetBAsyncSendSwapReserved=%s WS_fallback=%s)",
                fallbackWs ? "websocket-fallback" : "unified-cnet", cnetSendSource, cnetSwapSource,
                fallbackWs ? (wsHooked ? "armed" : "missing") : "skipped");
}

} // namespace ac::hooks
