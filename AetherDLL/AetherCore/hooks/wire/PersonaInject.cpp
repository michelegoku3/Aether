#include "pch.h"
#include "hooks/wire/PersonaInject.h"

#include <cstring>
#include <string>
#include <vector>

#include "scripting/LuaData.h"
#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "utils/GameNameResolver.h"

#include "steam_messages.pb.h"

namespace ac::hooks::PersonaInject {
namespace {

constexpr const char* kModule = "Wire.PersonaInject";
constexpr std::int32_t kNoChange = -1;
constexpr std::uint32_t kMaxPacket =
    sizeof(steam::MsgHdr) + constants::kWireMaxHeaderBytes + constants::kWireMaxBodyBytes;

CMsgClientPersonaState::Friend* FindSelf(CMsgClientPersonaState& msg, std::uint64_t selfId) {
    if (selfId == 0) return nullptr;
    for (int i = 0; i < msg.friends_size(); ++i) {
        auto* f = msg.mutable_friends(i);
        if (f->has_friendid() && f->friendid() == selfId) return f;
    }
    return nullptr;
}

bool BuildInjectLocked(steam::AppId appId) {
    // Caller holds presence.mutex.
    auto& pr = g_state.presence;
    if (!pr.haveSelfTemplate || pr.selfSteamId == 0) return false;

    CMsgClientPersonaState msg;
    if (!msg.ParseFromArray(pr.selfBody.data(), static_cast<int>(pr.selfBody.size()))) {
        return false;
    }
    auto* self = FindSelf(msg, pr.selfSteamId);
    if (!self) return false;

    // Copy KVs under the lock; name lookup (ForApp) uses its own cache mutex.
    std::vector<std::pair<std::string, std::string>> kvsCopy;
    if (appId != 0) {
        auto it = pr.rpKvs.find(appId);
        if (it != pr.rpKvs.end()) kvsCopy = it->second;
    }

    if (appId == 0) {
        self->clear_game_played_app_id();
        self->clear_gameid();
        self->clear_game_name();
        self->clear_rich_presence();
        msg.set_status_flags(msg.status_flags() | constants::kStatusFlagRichPresence);
    } else {
        self->set_game_played_app_id(appId);
        self->set_gameid(static_cast<std::uint64_t>(appId));
        // Name lookup does not take presence.mutex.
        const std::string name = gamename::ForApp(appId);
        if (!name.empty()) self->set_game_name(name);
        self->clear_rich_presence();
        if (!kvsCopy.empty()) {
            for (const auto& kv : kvsCopy) {
                auto* out = self->add_rich_presence();
                out->set_key(kv.first);
                out->set_value(kv.second);
            }
            msg.set_status_flags(msg.status_flags() | constants::kStatusFlagRichPresence);
        } else {
            msg.set_status_flags(msg.status_flags() & ~constants::kStatusFlagRichPresence);
        }
    }

    const std::uint32_t bodyLen = static_cast<std::uint32_t>(msg.ByteSizeLong());
    const std::uint32_t hdrLen = static_cast<std::uint32_t>(pr.selfHdr.size());
    const std::uint32_t total = sizeof(steam::MsgHdr) + hdrLen + bodyLen;
    if (bodyLen > constants::kWireMaxBodyBytes || total > kMaxPacket) {
        ++pr.injectBuildFailCount;
        AC_LOG_WARN(kModule, "Inject packet too large (%u bytes).", total);
        return false;
    }

    pr.stagedPacket.resize(total);
    auto* mhdr = reinterpret_cast<steam::MsgHdr*>(pr.stagedPacket.data());
    mhdr->eMsg = constants::emsg::kClientPersonaState | steam::kMsgHdrProtoFlag;
    mhdr->headerLength = hdrLen;
    if (hdrLen) {
        std::memcpy(pr.stagedPacket.data() + sizeof(steam::MsgHdr), pr.selfHdr.data(), hdrLen);
    }
    if (!msg.SerializeToArray(pr.stagedPacket.data() + sizeof(steam::MsgHdr) + hdrLen,
                              static_cast<int>(bodyLen))) {
        ++pr.injectBuildFailCount;
        pr.stagedPacket.clear();
        return false;
    }
    pr.injectPending = true;
    AC_LOG_INFO_ONCE(kModule, "Staged PersonaState inject for appid=%u (%u bytes).", appId, total);
    return true;
}

}  // namespace

void SetPlayingApp(steam::AppId appId, bool forceRestage) {
    if (!g_state.settings.presenceInjectLocal && appId != 0) {
        // Still allow clear (appId==0) so toggles clean up.
        std::lock_guard<std::mutex> lock(g_state.presence.mutex);
        if (g_state.presence.playingAppId == 0) return;
        g_state.presence.playingAppId = 0;
        BuildInjectLocked(0);
        return;
    }

    std::lock_guard<std::mutex> lock(g_state.presence.mutex);
    if (!forceRestage && g_state.presence.playingAppId == appId) return;
    g_state.presence.playingAppId = appId;
    if (forceRestage) {
        AC_LOG_INFO_ONCE(kModule, "Playing app -> %u (restage).", appId);
    } else {
        AC_LOG_INFO(kModule, "Playing app -> %u.", appId);
    }
    BuildInjectLocked(appId);
}

steam::AppId PlayingApp() {
    std::lock_guard<std::mutex> lock(g_state.presence.mutex);
    return g_state.presence.playingAppId;
}

std::int32_t OnPersonaStateRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientPersonaState msg;
    if (!msg.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) return kNoChange;

    bool changed = false;
    steam::AppId playing = 0;
    std::uint64_t selfId = 0;
    const bool ofPersonaPatch = g_state.settings.presenceOnlineFixPersonaPatch;
    const steam::AppId ofReal = g_state.onlineFixRealAppId.load();

    {
        std::lock_guard<std::mutex> lock(g_state.presence.mutex);
        auto& pr = g_state.presence;
        selfId = pr.selfSteamId;
        playing = pr.playingAppId;

        // Cache self template from any push that includes our friend entry.
        CMsgClientPersonaState::Friend* self = FindSelf(msg, selfId);
        if (self && frame.headerLen <= constants::kWireMaxHeaderBytes &&
            frame.bodyLen <= constants::kWireMaxBodyBytes) {
            pr.selfHdr.assign(frame.header, frame.header + frame.headerLen);
            pr.selfBody.assign(frame.body, frame.body + frame.bodyLen);
            pr.haveSelfTemplate = !pr.selfHdr.empty() && !pr.selfBody.empty();
        }

        // In-place self patch so periodic server pushes cannot wipe inject.
        if (playing != 0 && g_state.settings.presenceInjectLocal) {
            self = FindSelf(msg, selfId);
            if (self) {
                // Apply without re-entering mutex: copy kvs first.
                std::vector<std::pair<std::string, std::string>> kvsCopy;
                auto it = pr.rpKvs.find(playing);
                if (it != pr.rpKvs.end()) kvsCopy = it->second;

                self->set_game_played_app_id(playing);
                self->set_gameid(static_cast<std::uint64_t>(playing));
                // Name outside: drop lock first? ForApp is independent.
                // We'll set name after unlock for cleanliness — set placeholder now.
                self->clear_rich_presence();
                if (!kvsCopy.empty()) {
                    for (const auto& kv : kvsCopy) {
                        auto* kvOut = self->add_rich_presence();
                        kvOut->set_key(kv.first);
                        kvOut->set_value(kv.second);
                    }
                    msg.set_status_flags(msg.status_flags() | constants::kStatusFlagRichPresence);
                } else {
                    msg.set_status_flags(msg.status_flags() & ~constants::kStatusFlagRichPresence);
                }
                changed = true;
            }
        }
    }

    if (changed && playing != 0) {
        // Fill name without holding presence.mutex.
        for (int i = 0; i < msg.friends_size(); ++i) {
            auto* f = msg.mutable_friends(i);
            if (selfId && f->has_friendid() && f->friendid() == selfId) {
                const std::string name = gamename::ForApp(playing);
                if (!name.empty()) f->set_game_name(name);
                break;
            }
        }
    }

    // OnlineFix: patch any friend entry still showing Spacewar (local view).
    if (ofPersonaPatch && ofReal != 0 && luadata::IsConfigured(ofReal)) {
        const std::string name = gamename::ForApp(ofReal);
        for (int i = 0; i < msg.friends_size(); ++i) {
            auto* f = msg.mutable_friends(i);
            if (static_cast<steam::AppId>(f->game_played_app_id()) != constants::kSpacewarAppId) {
                continue;
            }
            f->set_game_played_app_id(ofReal);
            f->set_gameid(static_cast<std::uint64_t>(ofReal));
            if (!name.empty()) f->set_game_name(name);
            changed = true;
            AC_LOG_INFO_ONCE(kModule, "Patched friend %llu: 480 -> %u.",
                        static_cast<unsigned long long>(f->friendid()), ofReal);
        }
    }

    if (!changed) return kNoChange;

    const std::uint32_t size = static_cast<std::uint32_t>(msg.ByteSizeLong());
    if (size > outCap || !msg.SerializeToArray(out, static_cast<int>(outCap))) {
        AC_LOG_WARN(kModule, "PersonaState rewrite too large (%u bytes).", size);
        return kNoChange;
    }
    return static_cast<std::int32_t>(size);
}

void TryDeliver(void* recvThis, steam::CNetPacket* carrier,
                void* (*oRecvPkt)(void*, steam::CNetPacket*)) {
    if (!carrier || !oRecvPkt) return;

    std::vector<std::uint8_t> staged;
    {
        std::lock_guard<std::mutex> lock(g_state.presence.mutex);
        if (!g_state.presence.injectPending || g_state.presence.stagedPacket.empty()) return;
        staged.swap(g_state.presence.stagedPacket);
        g_state.presence.injectPending = false;
    }

    std::uint8_t* origData = carrier->data;
    const std::uint32_t origLen = carrier->dataLen;
    carrier->data = staged.data();
    carrier->dataLen = static_cast<std::uint32_t>(staged.size());
    oRecvPkt(recvThis, carrier);
    carrier->data = origData;
    carrier->dataLen = origLen;

    {
        std::lock_guard<std::mutex> lock(g_state.presence.mutex);
        ++g_state.presence.injectDeliverCount;
    }
    AC_LOG_INFO_ONCE(kModule, "Delivered staged PersonaState (%zu bytes).", staged.size());
}

void Reset() {
    std::lock_guard<std::mutex> lock(g_state.presence.mutex);
    g_state.presence.playingAppId = 0;
    g_state.presence.injectPending = false;
    g_state.presence.stagedPacket.clear();
    g_state.presence.haveSelfTemplate = false;
    g_state.presence.selfHdr.clear();
    g_state.presence.selfBody.clear();
    g_state.presence.rpKvs.clear();
}

}  // namespace ac::hooks::PersonaInject
