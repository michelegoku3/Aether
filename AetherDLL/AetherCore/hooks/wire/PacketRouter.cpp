#include "pch.h"
#include "hooks/wire/PacketRouter.h"

#include <array>
#include <cstdint>
#include <cstring>
#include <string>
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

std::int32_t DispatchSend(const WireFrame& f) {
    EnsureScratch();
    switch (f.eMsg) {
        case emsg::kClientPICSProductInfoRequest:
            return AccessToken::HandleSend(f, t_scratchBody.data(), kWireMaxBodyBytes);
        case emsg::kClientGamesPlayed:
        case emsg::kClientGamesPlayedWithDataBlob:
            return GamesPlayed::HandleSend(f, t_scratchBody.data(), kWireMaxBodyBytes);
        case emsg::kClientRichPresenceUpload:
            return GamesPlayed::HandleRichPresenceUpload(f);
        case emsg::kServiceMethodCallFromClient: {
            std::string job;
            if (ServiceJobName(f, job) &&
                FnvHash(job.c_str()) == job_hash::kGetManifestRequestCode) {
                return ManifestBridge::HandleSend(f);
            }
            return kNoChange;
        }
        default:
            AC_LOG_TRACE_ONCE(kModule, "Pass-through send frame eMsg=%u bodyLen=%u.", f.eMsg, f.bodyLen);
            return kNoChange;
    }
}

using BBuildAndAsyncSendFrame_t = bool (*)(void*, steam::EWebSocketOpCode, std::uint8_t*,
                                           std::uint32_t);
using RecvPkt_t = void* (*)(void*, steam::CNetPacket*);

BBuildAndAsyncSendFrame_t o_BBuildAndAsyncSendFrame = nullptr;
RecvPkt_t o_RecvPkt = nullptr;

bool h_BBuildAndAsyncSendFrame(void* obj, steam::EWebSocketOpCode opcode, std::uint8_t* data,
                               std::uint32_t len) {
    if (opcode != steam::k_eWebSocketOpCode_Binary) {
        return o_BBuildAndAsyncSendFrame(obj, opcode, data, len);
    }

    WireFrame f;
    if (DecodeFrame(data, len, f)) {
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
    switch (f.eMsg) {
        case emsg::kServiceMethodResponse: {
            std::string job;
            if (!ServiceJobName(f, job)) return kNoChange;
            std::uint32_t h = FnvHash(job.c_str());
            if (h == job_hash::kNotifyRunningApps) {
                return FamilySharing::ClearBody();
            }
            if (h == job_hash::kGetManifestRequestCode) {
                return ManifestBridge::HandleRecv(f, t_scratchBody.data(), kWireMaxBodyBytes,
                                                  t_scratchHeader.data(), kWireMaxHeaderBytes,
                                                  &t_recvHeaderLen);
            }
            return kNoChange;
        }
        case emsg::kClientPersonaState:
            return PersonaInject::OnPersonaStateRecv(f, t_scratchBody.data(), kWireMaxBodyBytes);
        case emsg::kClientRequestEncryptedAppTicketResponse:
            return EticketModule::HandleRecv(f, t_scratchBody.data(), kWireMaxBodyBytes);
        case emsg::kClientSharedLibraryLockStatus:
        case emsg::kClientSharedLibraryStopPlaying:
            return FamilySharing::ClearBody();
        default:
            AC_LOG_TRACE_ONCE(kModule, "Pass-through recv frame eMsg=%u bodyLen=%u.", f.eMsg, f.bodyLen);
            return kNoChange;
    }
}

void* h_RecvPkt(void* self, CNetPacket* packet) {
    if (packet) {
        WireFrame f;
        if (DecodeFrame(packet->data, packet->dataLen, f) && f.eMsg != emsg::kMulti) {
            std::int32_t newBodyLen = DispatchRecv(f);

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
            } else if (newBodyLen == 0) {
                packet->dataLen = sizeof(MsgHdr) + f.headerLen;
            } else if (newBodyLen > 0) {
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

        PersonaInject::TryDeliver(self, packet, o_RecvPkt);
    }
    return o_RecvPkt(self, packet);
}

}  // namespace

void RegisterPacketRouter(HMODULE diversion) {
    if (!diversion) {
        AC_LOG_ERROR(kModule, "Diversion module not loaded.");
        return;
    }
    AC_LOG_INFO(kModule, "Registering packet router.");

    g_state.hookManager.TryHook("BBuildAndAsyncSendFrame", "steamclient", diversion,
                                o_BBuildAndAsyncSendFrame, h_BBuildAndAsyncSendFrame);
    g_state.hookManager.TryHook("RecvPkt", "steamclient", diversion, o_RecvPkt, h_RecvPkt);
}

}  // namespace ac::hooks
