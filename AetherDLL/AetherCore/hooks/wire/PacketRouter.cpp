#include "pch.h"
#include "hooks/wire/PacketRouter.h"

#include <array>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/HookManager.h"
#include "core/Logger.h"
#include "hooks/wire/AccessTokenModule.h"
#include "hooks/wire/EticketModule.h"
#include "hooks/wire/FamilySharingModule.h"
#include "hooks/wire/GamesPlayedModule.h"
#include "hooks/wire/ManifestBridge.h"
#include "hooks/wire/PersonaInject.h"
#include "hooks/wire/AchievementModule.h"

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
            // calls on the same thread.
            thread_local Pool t_pool;
            auto& buf = t_pool.slots[t_pool.next++ % kWirePoolSlots];
            buf.resize(size);
            return buf.data();
        }

        thread_local std::vector<std::uint8_t> t_scratchBody;
        thread_local std::vector<std::uint8_t> t_scratchHeader;

        void EnsureScratch() {
            if (t_scratchBody.size() < kWireMaxBodyBytes) t_scratchBody.resize(kWireMaxBodyBytes);
            if (t_scratchHeader.size() < kWireMaxHeaderBytes) t_scratchHeader.resize(kWireMaxHeaderBytes);
        }

        bool DecodeFrame(const std::uint8_t* data, std::uint32_t len, WireFrame& out) {
            if (!data || len < sizeof(MsgHdr)) {
                AC_LOG_TRACE_ONCE(kModule, "DecodeFrame failed: buffer null or size %u below MsgHdr size.", len);
                return false;
            }
            const auto* hdr = reinterpret_cast<const MsgHdr*>(data);
            // Skip non-protobuf packets — the headerLength field overlaps with payload
            // data and contains garbage values. Matches LumaCore's ParsePacket guard.
            if (!(hdr->eMsg & kMsgHdrProtoFlag)) {
                return false;
            }
            const std::uint32_t eMsg = hdr->eMsg & ~kMsgHdrProtoFlag;
            const std::uint32_t headerLen = hdr->headerLength;
            if (sizeof(MsgHdr) + headerLen > len) {
                AC_LOG_TRACE_ONCE(kModule, "DecodeFrame failed: header length %u exceeds total frame length %u.", headerLen, len);
                return false;
            }
            out.eMsg = eMsg;
            out.header = data + sizeof(MsgHdr);
            out.headerLen = headerLen;
            out.body = data + sizeof(MsgHdr) + headerLen;
            out.bodyLen = len - sizeof(MsgHdr) - headerLen;
            return true;
        }

        bool ServiceJobName(const WireFrame& f, std::string& out) {
            CMsgProtoBufHeader hdr;
            if (!hdr.ParseFromArray(f.header, static_cast<int>(f.headerLen))) return false;
            if (!hdr.has_target_job_name()) return false;
            out = hdr.target_job_name();
            return true;
        }

        bool ServiceJobIdSource(const WireFrame& f, std::uint64_t& out) {
            CMsgProtoBufHeader hdr;
            if (!hdr.ParseFromArray(f.header, static_cast<int>(f.headerLen))) return false;
            if (!hdr.has_jobid_source()) return false;
            out = hdr.jobid_source();
            return true;
        }

        // [DIAG]/flight-recorder: correlazione jobid_source -> target_job_name
        // sui send 151, risolta jobid_target -> nome sui recv 147 (le risposte
        // non portano il nome del servizio, solo il job id della richiesta).
        std::mutex s_jobNameMutex;
        std::unordered_map<std::uint64_t, std::string> s_jobNames;

        void TrackServiceJob(const WireFrame& f) {
            std::uint64_t id = 0;
            if (!ServiceJobIdSource(f, id)) return;
            std::string name;
            if (!ServiceJobName(f, name)) return;
            std::lock_guard<std::mutex> lk(s_jobNameMutex);
            if (s_jobNames.size() > 512) s_jobNames.erase(s_jobNames.begin());
            s_jobNames.emplace(id, std::move(name));
        }

        std::string ResolveServiceJob(std::uint64_t jobIdTarget) {
            std::lock_guard<std::mutex> lk(s_jobNameMutex);
            auto it = s_jobNames.find(jobIdTarget);
            if (it == s_jobNames.end()) return {};
            std::string out = std::move(it->second);
            s_jobNames.erase(it);
            return out;
        }

        // [DIAG] snapshot eresult+jobid_target dell'header di una risposta (147).
        std::int32_t RecvResponseMeta(const WireFrame& f, std::uint64_t* jobIdTarget) {
            CMsgProtoBufHeader hdr;
            if (!hdr.ParseFromArray(f.header, static_cast<int>(f.headerLen))) return -1;
            if (jobIdTarget && hdr.has_jobid_target()) *jobIdTarget = hdr.jobid_target();
            return hdr.has_eresult() ? hdr.eresult() : -1;
        }

        // Trace per-frame bidirezionale (flight-recorder): un frame = una riga,
        // cosi' l'ultima riga del log e' SEMPRE l'ultimo frame prima del crash.
        void TraceFrame(const char* dir, const WireFrame& f) {
            AC_LOG_TRACE(kModule, "%s eMsg=%u hLen=%u bLen=%u", dir, f.eMsg, f.headerLen,
                         f.bodyLen);
        }

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
            case emsg::kServiceMethodCallFromClient: {
                std::string job;
                if (ServiceJobName(f, job)) {
                    TrackServiceJob(f);  // [DIAG] flight-recorder jobid->nome
                    std::uint32_t h = FnvHash(job.c_str());
                    if (h == job_hash::kGetManifestRequestCode) {
                        return ManifestBridge::HandleSend(f);
                    }
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

        using BBuildAndAsyncSendFrame_t = bool (*)(void*, steam::EWebSocketOpCode, std::uint8_t*,
            std::uint32_t);
        using RecvPkt_t = void* (*)(void*, steam::CNetPacket*);

        BBuildAndAsyncSendFrame_t o_BBuildAndAsyncSendFrame = nullptr;
        RecvPkt_t o_RecvPkt = nullptr;

        // Connection + proto header captured from real outbound traffic, so
        // SendClientFrame can originate a frame on the same socket/session
        // (used to ask the CM for AppInfo records the local cache lacks).
        std::mutex s_originMutex;
        void* s_originObj = nullptr;
        std::vector<std::uint8_t> s_originHeader;

        bool h_BBuildAndAsyncSendFrame(void* obj, steam::EWebSocketOpCode opcode, std::uint8_t* data,
            std::uint32_t len) {
            if (opcode != steam::k_eWebSocketOpCode_Binary) {
                return o_BBuildAndAsyncSendFrame(obj, opcode, data, len);
            }

            WireFrame f;
            if (!DecodeFrame(data, len, f)) {
                if (data && len >= sizeof(MsgHdr)) {
                    const auto* hdr = reinterpret_cast<const MsgHdr*>(data);
                    const std::uint32_t raw = hdr->eMsg;
                    AC_LOG_TRACE(kModule, "send undecoded eMsg=%u proto=%d len=%u",
                                 raw & ~kMsgHdrProtoFlag, (raw & kMsgHdrProtoFlag) ? 1 : 0, len);
                }
            } else {
                if (f.headerLen > 0 && f.headerLen <= kWireMaxHeaderBytes) {
                    std::lock_guard<std::mutex> lk(s_originMutex);
                    s_originObj = obj;
                    s_originHeader.assign(f.header, f.header + f.headerLen);
                }
                std::int32_t newBodyLen = DispatchSend(f);
                if (newBodyLen >= 0) {
                    const std::uint32_t newSize = sizeof(MsgHdr) + f.headerLen + newBodyLen;
                    std::uint8_t* buf = PoolSlot(newSize);
                    if (!buf) return o_BBuildAndAsyncSendFrame(obj, opcode, data, len);
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
                // [DIAG] flight-recorder: OGGI una riga per OGNI risposta
                // servizio, col nome ricostruito via jobid tracciato al send.
                {
                    std::uint64_t jt = 0;
                    const std::int32_t er = RecvResponseMeta(f, &jt);
                    const std::string jname = jt ? ResolveServiceJob(jt) : std::string();
                    AC_LOG_TRACE(kModule,
                                 "[DIAG] service recv name='%s' jobid=%llu eresult=%d bLen=%u",
                                 jname.empty() ? "?" : jname.c_str(),
                                 static_cast<unsigned long long>(jt), er, f.bodyLen);
                }
                std::string job;
                if (!ServiceJobName(f, job)) return kNoChange;
                std::uint32_t h = FnvHash(job.c_str());
                if (h == job_hash::kNotifyRunningApps) {
                    return FamilySharing::ShouldSuppress()
                        ? FamilySharing::ClearBody()
                        : kNoChange;
                }
                if (h == job_hash::kGetManifestRequestCode) {
                    return ManifestBridge::HandleRecv(f, t_scratchBody.data(), kWireMaxBodyBytes,
                        t_scratchHeader.data(), kWireMaxHeaderBytes,
                        &t_recvHeaderLen);
                }
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
                AC_LOG_INFO(kModule,
                            "[DIAG] SharedLibrary msg eMsg=%u bodyLen=%u suppress=%d",
                            f.eMsg, f.bodyLen, FamilySharing::ShouldSuppress() ? 1 : 0);
                return FamilySharing::ShouldSuppress()
                    ? FamilySharing::ClearBody()
                    : kNoChange;
            default:
                return kNoChange;
            }
        }

        void* h_RecvPkt(void* self, CNetPacket* packet) {
            if (packet) {
                WireFrame f;
                if (DecodeFrame(packet->data, packet->dataLen, f)) {
                    if (f.eMsg == emsg::kMulti) {
                        // [DIAG] flight-recorder: i frame Multi non entrano nel
                        // dispatch, ma devono restare visibili nel trace (con la
                        // firma dei primi byte del body, per riconoscere lo zlib
                        // di un payload compresso se servira' decodificarli).
                        static constexpr char kHex[] = "0123456789ABCDEF";
                        char head[8 * 2 + 1] = {};
                        const std::uint32_t n = f.bodyLen < 8 ? f.bodyLen : 8;
                        for (std::uint32_t i = 0; i < n; ++i) {
                            head[i * 2] = kHex[(f.body[i] >> 4) & 0xF];
                            head[i * 2 + 1] = kHex[f.body[i] & 0xF];
                        }
                        AC_LOG_TRACE(kModule, "recv eMsg=1 (Multi) bLen=%u head8=%s", f.bodyLen, head);
                    } else {
                    const std::int32_t newBodyLen = DispatchRecv(f);

                    if (t_recvHeaderLen >= 0 && newBodyLen >= 0) {
                        const std::uint32_t newSize = sizeof(MsgHdr) + t_recvHeaderLen + newBodyLen;
                        std::uint8_t* buf = PoolSlot(newSize);
                        if (buf) {
                            std::memcpy(buf, packet->data, sizeof(MsgHdr));
                            reinterpret_cast<MsgHdr*>(buf)->headerLength = t_recvHeaderLen;
                            std::memcpy(buf + sizeof(MsgHdr), t_scratchHeader.data(), t_recvHeaderLen);
                            std::memcpy(buf + sizeof(MsgHdr) + t_recvHeaderLen, t_scratchBody.data(),
                                newBodyLen);
                            packet->data = buf;
                            packet->dataLen = newSize;
                        }
                    }
                    else if (newBodyLen == 0) {
                        packet->dataLen = sizeof(MsgHdr) + f.headerLen;
                    }
                    else if (newBodyLen > 0) {
                        const std::uint32_t newSize = sizeof(MsgHdr) + f.headerLen + newBodyLen;
                        std::uint8_t* buf = PoolSlot(newSize);
                        if (buf) {
                            std::memcpy(buf, packet->data, sizeof(MsgHdr));
                            reinterpret_cast<MsgHdr*>(buf)->headerLength = f.headerLen;
                            std::memcpy(buf + sizeof(MsgHdr), f.header, f.headerLen);
                            std::memcpy(buf + sizeof(MsgHdr) + f.headerLen, t_scratchBody.data(),
                                newBodyLen);
                            packet->data = buf;
                            packet->dataLen = newSize;
                        }
                    }
                    }
                }

                PersonaInject::TryDeliver(self, packet, o_RecvPkt);
            }
            return o_RecvPkt(self, packet);
        }

    }  // namespace

    bool SendClientFrame(std::uint32_t eMsg, const std::uint8_t* body, std::uint32_t bodyLen) {
        if (!o_BBuildAndAsyncSendFrame) {
            AC_LOG_WARN(kModule, "SendClientFrame: send hook not installed.");
            return false;
        }
        if (bodyLen > kWireMaxBodyBytes) return false;

        void* obj = nullptr;
        std::vector<std::uint8_t> hdrBytes;
        {
            std::lock_guard<std::mutex> lk(s_originMutex);
            obj = s_originObj;
            hdrBytes = s_originHeader;
        }
        if (!obj || hdrBytes.empty()) {
            AC_LOG_WARN(kModule, "SendClientFrame: no captured connection yet.");
            return false;
        }

        // Re-serialize the captured header with fresh job ids: keep steamid and
        // client_sessionid, drop any correlation belonging to the borrowed message.
        CMsgProtoBufHeader ph;
        if (!ph.ParseFromArray(hdrBytes.data(), static_cast<int>(hdrBytes.size()))) {
            AC_LOG_WARN(kModule, "SendClientFrame: captured header unparseable.");
            return false;
        }
        ph.clear_jobid_source();
        ph.clear_jobid_target();
        ph.clear_target_job_name();
        ph.clear_eresult();

        std::string hdrOut;
        if (!ph.SerializeToString(&hdrOut) || hdrOut.size() > kWireMaxHeaderBytes) return false;

        const std::uint32_t total =
            sizeof(MsgHdr) + static_cast<std::uint32_t>(hdrOut.size()) + bodyLen;
        std::vector<std::uint8_t> frame(total);
        auto* mh = reinterpret_cast<MsgHdr*>(frame.data());
        mh->eMsg = eMsg | kMsgHdrProtoFlag;
        mh->headerLength = static_cast<std::uint32_t>(hdrOut.size());
        std::memcpy(frame.data() + sizeof(MsgHdr), hdrOut.data(), hdrOut.size());
        if (bodyLen > 0) {
            std::memcpy(frame.data() + sizeof(MsgHdr) + hdrOut.size(), body, bodyLen);
        }

        const bool ok = o_BBuildAndAsyncSendFrame(obj, steam::k_eWebSocketOpCode_Binary,
                                                  frame.data(), total);
        AC_LOG_INFO(kModule,
                    "[DIAG] SendClientFrame eMsg=%u hdr=%zu body=%u steamid=%llu sess=%d -> %s",
                    eMsg, hdrOut.size(), bodyLen,
                    static_cast<unsigned long long>(ph.steamid()), ph.client_sessionid(),
                    ok ? "ok" : "FAILED");
        return ok;
    }

    void RegisterPacketRouter(HMODULE diversion) {
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
                    g_state.settings.presenceInjectLocal ? 1 : 0,
                    g_state.settings.presenceAlwaysExtraInfo ? 1 : 0,
                    g_state.settings.presenceShowOnlineBroadcast ? 1 : 0,
                    g_state.settings.presenceFriendAppIdFromName ? 1 : 0,
                    g_state.settings.presenceAppIdBlob ? 1 : 0,
                    g_state.settings.presenceSuffixInvisible ? 1 : 0);

        g_state.hookManager.TryHook("BBuildAndAsyncSendFrame", "steamclient", diversion,
            o_BBuildAndAsyncSendFrame, h_BBuildAndAsyncSendFrame);
        g_state.hookManager.TryHook("RecvPkt", "steamclient", diversion, o_RecvPkt, h_RecvPkt);
    }

}  // namespace ac::hooks
