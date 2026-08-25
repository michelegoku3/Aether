#include "pch.h"
#include "hooks/wire/PersonaInject.h"

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Constants.h"
#include "core/Logger.h"
#include "hooks/wire/PacketRouter.h"
#include "scripting/LuaData.h"
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

// The sender hides the session appid in game_extra_info; the CM recycles that
// text into Friend.game_name for masked sessions (measured). Some clients
// also ship it as a rich-presence KV — keep both read paths.
std::string ExtraInfoKV(const CMsgClientPersonaState::Friend& f) {
    for (const auto& kv : f.rich_presence()) {
        if (kv.has_key() && kv.key() == "game_extra_info" && kv.has_value()) {
            return kv.value();
        }
    }
    return {};
}

// Parses the "<name> | <appid>" marker appended by Aether senders
// (kExtraInfoAppIdSep). Returns the appid and writes the clean display name
// back to nameOut; returns 0 when the text carries no valid suffix.
steam::AppId AppIdFromAsciiSuffix(const std::string& text, std::string& nameOut) {
    nameOut = text;
    const std::string sep = constants::kExtraInfoAppIdSep;
    const std::size_t pos = text.rfind(sep);
    if (pos == std::string::npos) return 0;

    const std::string tail = text.substr(pos + sep.size());
    if (tail.empty() || tail.size() > 8) return 0;
    for (const char c : tail) {
        if (c < '0' || c > '9') return 0;
    }
    const unsigned long v = std::strtoul(tail.c_str(), nullptr, 10);
    if (v == 0 || v > constants::kGameIdAppIdMask || v == constants::kSpacewarAppId) {
        return 0;
    }
    nameOut = text.substr(0, pos);
    return static_cast<steam::AppId>(v);
}

// Invisible channel (constants::kExtraInfoInvisible*): exact tail of the text
// must be U+200B (E2 80 8B) followed by 6 bytes-triples 0xEE 0xB8 (0x80|nib).
// 24-bit value, big-endian nibbles. Vanilla renderers draw nothing.
steam::AppId AppIdFromInvisibleSuffix(const std::string& text, std::string& nameOut) {
    nameOut = text;
    constexpr std::size_t markLen = sizeof(constants::kExtraInfoInvisibleMark) - 1;  // 3
    constexpr std::size_t digits = constants::kExtraInfoInvisibleDigits;             // 6
    constexpr std::size_t tailLen = markLen + digits * 3;                            // 21
    if (text.size() < tailLen) return 0;
    const std::size_t markPos = text.size() - tailLen;
    if (text.compare(markPos, markLen, constants::kExtraInfoInvisibleMark) != 0) return 0;

    std::uint32_t v = 0;
    for (std::size_t i = 0; i < digits; ++i) {
        const std::size_t o = markPos + markLen + i * 3;
        const unsigned char b0 = static_cast<unsigned char>(text[o]);
        const unsigned char b1 = static_cast<unsigned char>(text[o + 1]);
        const unsigned char b2 = static_cast<unsigned char>(text[o + 2]);
        if (b0 != 0xEE || b1 != 0xB8 || b2 < 0x80 || b2 > 0x8F) return 0;
        v = (v << 4) | (b2 & 0xF);
    }
    if (v == 0 || v == constants::kSpacewarAppId) return 0;
    nameOut = text.substr(0, markPos);
    return static_cast<steam::AppId>(v);
}

// Lobby/duo corroboration key: 'steam_player_group' is present in rich
// presence whenever a client hosts or joins a lobby. Returns 0 when absent.
std::uint64_t LobbyGroupId(const CMsgClientPersonaState::Friend& f) {
    for (const auto& kv : f.rich_presence()) {
        if (kv.has_key() && kv.key() == "steam_player_group" && kv.has_value()) {
            return std::strtoull(kv.value().c_str(), nullptr, 10);
        }
    }
    return 0;
}

// Our own current lobby, mirrored from the SELF persona entry on every push.
// 0 = not in any lobby. Shared-lobby is the ONLY safe corroboration for the
// legacy local-session attribution below.
std::atomic<std::uint64_t> g_selfLobby{0};

// Combined parse: ASCII suffix (legacy senders) first, invisible channel next.
steam::AppId AppIdFromSuffix(const std::string& text, std::string& nameOut) {
    if (const steam::AppId a = AppIdFromAsciiSuffix(text, nameOut)) return a;
    return AppIdFromInvisibleSuffix(text, nameOut);
}

// Preferred channel (docs/05 §10): build fix5+ senders hide the appid in
// games_played.game_data_blob; the CM recycles it into
// Friend.game_data_blob for masked sessions. Raw bytes — no client UI ever
// renders them, vanilla friends see only the plain name from game_name.
steam::AppId AppIdFromBlob(const std::string& blob) {
    if (blob.size() != constants::kAppIdBlobLen) return 0;
    if (blob.compare(0, 4, constants::kAppIdBlobMagic, 4) != 0) return 0;
    if (static_cast<std::uint8_t>(blob[4]) != constants::kAppIdBlobVersion) return 0;
    const std::uint32_t v = static_cast<std::uint8_t>(blob[5]) |
                            (static_cast<std::uint8_t>(blob[6]) << 8) |
                            (static_cast<std::uint8_t>(blob[7]) << 16) |
                            (static_cast<std::uint32_t>(static_cast<std::uint8_t>(blob[8])) << 24);
    if (v == 0 || v > constants::kGameIdAppIdMask || v == constants::kSpacewarAppId) return 0;
    return static_cast<steam::AppId>(v);
}

// The friend-list icon is keyed on the appid inside the LOCAL AppInfo cache:
// when that cache cannot name the app at all the UI has nothing to draw, so
// ask the CM for the PICS record (product metadata is public, not
// license-gated; the response lands on the stock path and fills the cache).
void EnsureAppInfo(steam::AppId appId) {
    static std::mutex s_mutex;
    static std::unordered_set<steam::AppId> s_requested;
    {
        std::lock_guard<std::mutex> lk(s_mutex);
        if (!s_requested.insert(appId).second) return;  // one-shot per process
    }

    CMsgClientPICSProductInfoRequest req;
    auto* app = req.add_apps();
    app->set_appid(appId);
    std::string body;
    if (!req.SerializeToString(&body)) {
        AC_LOG_WARN(kModule, "PICS appinfo request serialize failed for app %u.", appId);
        return;
    }
    AC_LOG_INFO(kModule,
                "[DIAG] app %u missing from local AppInfo cache; requesting PICS "
                "product info (icon/title fill).",
                appId);

    // Deferred, off the RecvPkt callstack: SendClientFrame re-enters the raw
    // BBuildAndAsyncSendFrame; doing that synchronously from inside the RecvPkt
    // callback is an unproven path on this strict a build. A short detached
    // worker keeps the wire callback clean; the SendClientFrame INFO line
    // brackets the attempt exactly if it ever faults.
    std::thread([body = std::move(body), appId]() mutable {
        Sleep(50);
        AC_LOG_TRACE(kModule, "[DIAG] PICS appinfo send begin for app %u.", appId);
        SendClientFrame(constants::emsg::kClientPICSProductInfoRequest,
                        reinterpret_cast<const std::uint8_t*>(body.data()),
                        static_cast<std::uint32_t>(body.size()));
        AC_LOG_TRACE(kModule, "[DIAG] PICS appinfo send done for app %u.", appId);
    }).detach();
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

    // [DIAG] SERVER truth, pre-patch: the PersonaState the CM returns for OUR
    // own SteamID is exactly what gets broadcast to friends. app=<real> would
    // mean the CM accepts the id; app=480 is the masked baseline; the KV dump
    // answers whether game_extra_info also arrives as a rich-presence KV.
    {
        static std::atomic<std::uint32_t> s_lastServerApp{0xFFFFFFFFu};
        if (auto* srv = FindSelf(msg, selfId)) {
            const std::uint32_t app = srv->game_played_app_id();
            // Track our own lobby membership every push: it is the
            // corroboration key for the legacy local-session attribution.
            g_selfLobby.store(LobbyGroupId(*srv), std::memory_order_relaxed);
            if (s_lastServerApp.exchange(app) != app) {
                AC_LOG_INFO(kModule,
                            "[DIAG] SERVER self-push (pre-patch): app=%u gameid=%llu "
                            "name='%s' rp_kvs=%d status_flags=0x%X",
                            app, static_cast<unsigned long long>(srv->gameid()),
                            srv->has_game_name() ? srv->game_name().c_str() : "",
                            srv->rich_presence_size(), msg.status_flags());
            }
            for (int k = 0; k < srv->rich_presence_size(); ++k) {
                const auto& kv = srv->rich_presence(k);
                AC_LOG_INFO_ONCE(kModule, "[DIAG] SERVER rp kv: '%s' = '%s'",
                                 kv.key().c_str(), kv.value().c_str());
            }
        }

        // [DIAG] flight-recorder: per OGNI persona frame, logga ogni friend che
        // sta giocando (appid reale o 480), deduplicato solo sul cambio di stato.
        // E' l'unico modo di vedere QUANDO una voce amico (es. 1703340 reale)
        // raggiunge questa macchina: se la riga manca, il CM non l'ha mai
        // consegnata (o e' arrivata dentro un Multi, vedi trace eMsg=1).
        {
            static std::mutex s_friendDumpMutex;
            static std::unordered_map<std::uint64_t, std::string> s_friendState;
            std::lock_guard<std::mutex> lk(s_friendDumpMutex);
            AC_LOG_TRACE(kModule, "[DIAG] PERSONA frame: friends=%d bLen=%u",
                         msg.friends_size(), frame.bodyLen);
            for (int i = 0; i < msg.friends_size(); ++i) {
                const auto& f = msg.friends(i);
                const std::uint32_t app = f.game_played_app_id();
                const std::uint64_t gid = f.gameid();
                if (app == 0 && gid == 0 && !f.has_game_name()) continue;
                if (!f.has_friendid()) continue;
                const std::uint64_t fid = f.friendid();
                char state[256];
                std::snprintf(state, sizeof(state), "app=%u gid=%llu name='%s' kvs=%d", app,
                              static_cast<unsigned long long>(gid),
                              f.has_game_name() ? f.game_name().c_str() : "",
                              f.rich_presence_size());
                const bool isSelf = selfId != 0 && fid == selfId;
                auto it = s_friendState.find(fid);
                if (it == s_friendState.end() || it->second != state) {
                    s_friendState[fid] = state;
                    AC_LOG_INFO(kModule, "[DIAG] PERSONA friend %llu (%s): %s",
                                static_cast<unsigned long long>(fid), isSelf ? "SELF" : "FRIEND",
                                state);
                }
            }
        }
        for (int i = 0; i < msg.friends_size(); ++i) {
            const auto& f = msg.friends(i);
            if (selfId != 0 && f.has_friendid() && f.friendid() == selfId) continue;
            if (f.rich_presence_size() == 0) continue;
            for (int k = 0; k < f.rich_presence_size(); ++k) {
                const auto& kv = f.rich_presence(k);
                AC_LOG_INFO_ONCE(kModule, "[DIAG] FRIEND %llu rp kv: '%s' = '%s' (app=%u)",
                                 static_cast<unsigned long long>(f.friendid()),
                                 kv.key().c_str(), kv.value().c_str(),
                                 f.game_played_app_id());
            }
        }
    }

    // ---- Friend entry recovery (480 -> real appid) -------------------------
    // The CM never broadcasts an appid the sender has no license for
    // (measured, docs/04-showonline-plan.md). What it DOES broadcast reliably
    // is game_extra_info, recycled into Friend.game_name. Aether senders hide
    // the exact appid in that text ("<name> | <appid>"): recover it here, on
    // the viewer's machine — exact, language-independent, no shared .lua
    // needed. Fallbacks: configured-library title match, then — ONLY when the
    // friend shares OUR lobby — the local -onlinefix session id (see the
    // lobby guard at its use site, below).
    {
        const steam::AppId ofReal = g_state.onlineFixRealAppId.load();
        const bool legacyGate = g_state.settings.presenceOnlineFixPersonaPatch &&
                                ofReal != 0 && luadata::IsConfigured(ofReal);
        std::vector<steam::AppId> picsQueue;

        for (int i = 0; i < msg.friends_size(); ++i) {
            auto* f = msg.mutable_friends(i);
            if (static_cast<steam::AppId>(f->game_played_app_id()) != constants::kSpacewarAppId) {
                continue;
            }
            // Never touch our own entry: the local self-view belongs to
            // presenceInjectLocal, and rewriting the appid the server believes
            // WE are running can tear down the Spacewar session that backs
            // -onlinefix (measured regression, see docs/04-showonline-plan.md).
            if (selfId != 0 && f->has_friendid() && f->friendid() == selfId) {
                AC_LOG_INFO_ONCE(kModule,
                                 "[DIAG] self entry arrived as 480; left untouched "
                                 "(session bookkeeping).");
                continue;
            }

            std::string displayName;
            steam::AppId real = 0;
            const char* source = "extra_info";
            bool fromLocalSession = false;

            std::string extra = ExtraInfoKV(*f);
            if (extra.empty() && f->has_game_name()) extra = f->game_name();
            real = AppIdFromSuffix(extra, displayName);
            // fix5 channels (docs/05 §10): raw-bytes blob + plan B appid packed
            // in gid bits 32-63. One shot of per-friend DIAG proves WHAT the
            // CM actually relayed (field test 16:38 2026-08-24: neither the
            // blob nor high bits arrived from a fix5 build — decide channels
            // from hex/len here).
            if (real == 0 && f->has_game_data_blob()) {
                real = AppIdFromBlob(f->game_data_blob());
                if (real != 0) {
                    source = "blob";
                    displayName = extra;
                }
            }
            uint32_t gidHi = 0;
            if (f->has_gameid()) gidHi = static_cast<std::uint32_t>(f->gameid() >> 32);
            if (real == 0 && gidHi != 0 && gidHi <= constants::kGameIdAppIdMask &&
                gidHi != constants::kSpacewarAppId) {
                real = gidHi;
                source = "gameid";
                displayName = extra;
            }
            {
                static std::unordered_set<std::uint64_t> s_diagFriends;
                const std::uint64_t fid = f->has_friendid() ? f->friendid() : 0ull;
                if (s_diagFriends.insert(fid).second) {
                    char hex[32] = "-";
                    std::size_t blobLen = 0;
                    if (f->has_game_data_blob()) {
                        const std::string& blob = f->game_data_blob();
                        blobLen = blob.size();
                        for (std::size_t b = 0; b < blob.size() && b < 4; ++b) {
                            std::snprintf(hex + (b == 0 ? 0 : std::strlen(hex)), 4,
                                          "%s%02X", b == 0 ? "" : " ",
                                          static_cast<unsigned char>(blob[b]));
                        }
                    }
                    AC_LOG_INFO(kModule,
                                "[DIAG] mask friend %llu: gid=%llu gidHi=%u bloblen=%zu "
                                "blobhead=%s extra='%s'",
                                static_cast<unsigned long long>(fid),
                                static_cast<unsigned long long>(f->has_gameid() ? f->gameid() : 0ull),
                                gidHi, blobLen, hex, extra.c_str());
                }
            }

            if (real == 0 && g_state.settings.presenceFriendAppIdFromName &&
                f->has_game_name() && !f->game_name().empty()) {
                if (const steam::AppId byName = gamename::ResolveAppIdByName(f->game_name())) {
                    if (byName != constants::kSpacewarAppId) {
                        real = byName;
                        displayName = f->game_name();
                        source = "by name";
                    }
                }
            }

            // Legacy local-session fallback (pre-suffix behaviour, kept behind
            // onlinefix_persona_patch): "a 480-friend while I'm masked must be
            // my co-op partner in my game". UNGUARDED that guess is WRONG in
            // general — measured 2026-08-25: a friend in his own unrelated
            // masked session was displayed as playing OUR game ('MECCHA
            // CHAMELEON', never installed on his machine). Attribute ONLY when
            // the friend's lobby (steam_player_group KV) matches our own: a
            // shared lobby is the only corroboration that cannot lie, while a
            // bare 480 match fabricates ownership out of thin air.
            if (real == 0 && legacyGate) {
                const std::uint64_t selfLobby = g_selfLobby.load(std::memory_order_relaxed);
                const std::uint64_t friendLobby = LobbyGroupId(*f);
                if (selfLobby != 0 && friendLobby != 0 && friendLobby == selfLobby) {
                    real = ofReal;
                    displayName.clear();
                    source = "local session (same lobby)";
                    fromLocalSession = true;
                }
            }

            if (real == 0) {
                AC_LOG_DEBUG_ONCE(kModule,
                                  "Friend %llu shows 480 with no recoverable appid "
                                  "(no suffix, no blob, no gid-hi, no title match, no local session).",
                                  static_cast<unsigned long long>(f->friendid()));
                continue;
            }

            // Display name: the exact text relayed in extra_info comes first.
            // It is minted by the sender from ITS own local AppInfo cache
            // (fresh, already localized), so it beats re-resolving on this
            // machine. The legacy "local session" source keeps the live cache
            // lookup: that app is installed HERE, and probing the cache for a
            // locally-known appid is the path measured safe. Probing for an
            // appid this machine never had faults inside steamclient (measured
            // crash, 12:57 log) — never do it on the recovery path.
            std::string name = displayName;
            if (name.empty() && fromLocalSession) {
                name = gamename::ForApp(real);
            }

            f->set_game_played_app_id(real);
            f->set_gameid(static_cast<std::uint64_t>(real));
            if (!name.empty()) {
                f->set_game_name(name);
            } else {
                f->clear_game_name();
            }
            changed = true;
            AC_LOG_INFO_ONCE(kModule, "Patched friend %llu: 480 -> %u (%s).",
                             static_cast<unsigned long long>(f->friendid()), real, source);

            // The icon needs the FULL AppInfo record (clienticon), not just the
            // title: prime the local cache with one PICS request per appid per
            // process (see EnsureAppInfo). Skipping it for the local-session
            // source when the cache already knows the name (record present).
            if (!fromLocalSession || name.empty()) picsQueue.push_back(real);
        }

        for (const steam::AppId want : picsQueue) EnsureAppInfo(want);
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
