#include "pch.h"
#include "hooks/aetheronline/PresenceSession.h"

#include <atomic>
#include <mutex>

#include "core/Logger.h"

namespace ac::presence {
namespace {

constexpr const char* kModule = "Presence";

// One atomic word per field keeps the historical hot-path cost (a bare load)
// while Publish() guarantees the fields are stored as a group in a fixed
// order; the snapshot is the authoritative source for readers that need a
// consistent view of MORE than one field.
std::atomic<steam::AppId> s_realAppId{0};
std::atomic<steam::AppId> s_showOnlineAppId{0};
std::atomic<bool> s_spacewarSpoofExpected{false};

std::mutex s_mutex;
std::shared_ptr<const SessionSnapshot> s_snapshot =
    std::make_shared<const SessionSnapshot>();

void PublishLocked(SessionSnapshot next) {
    s_realAppId.store(next.realAppId, std::memory_order_release);
    s_showOnlineAppId.store(next.showOnlineAppId, std::memory_order_release);
    s_spacewarSpoofExpected.store(next.spacewarSpoofExpected, std::memory_order_release);
    s_snapshot = std::make_shared<const SessionSnapshot>(next);
}

}  // namespace

void Publish(SessionSnapshot next) {
    std::lock_guard<std::mutex> lock(s_mutex);
    const auto prev = s_snapshot;
    PublishLocked(next);
    AC_LOG_INFO(kModule,
                "Session published: mode=%s real=%u showonline=%u spoof_expected=%d "
                "(was: mode=%s real=%u showonline=%u).",
                next.ModeText(), next.realAppId, next.showOnlineAppId,
                next.spacewarSpoofExpected ? 1 : 0,
                prev->ModeText(), prev->realAppId, prev->showOnlineAppId);
}

void ClearShowOnline() {
    std::lock_guard<std::mutex> lock(s_mutex);
    if (s_snapshot->showOnlineAppId == 0) return;
    SessionSnapshot next = *s_snapshot;
    next.showOnlineAppId = 0;
    PublishLocked(next);
    AC_LOG_INFO(kModule,
                "ShowOnline cleared: a foreign Spacewar spoof owns the session "
                "(real=%u unchanged).",
                next.realAppId);
}

void EndSession() {
    std::lock_guard<std::mutex> lock(s_mutex);
    if (s_snapshot->realAppId == 0 && s_snapshot->showOnlineAppId == 0 &&
        !s_snapshot->spacewarSpoofExpected) {
        return;  // nothing was active; GamesPlayed empties arrive periodically
    }
    SessionSnapshot next;  // all zero/empty
    PublishLocked(next);
    AC_LOG_INFO(kModule, "Session ended: no running games reported; presence state reset.");
}

std::shared_ptr<const SessionSnapshot> Current() {
    std::lock_guard<std::mutex> lock(s_mutex);
    return s_snapshot;
}

steam::AppId RealAppId() { return s_realAppId.load(std::memory_order_acquire); }
steam::AppId ShowOnlineAppId() { return s_showOnlineAppId.load(std::memory_order_acquire); }
bool SpacewarSpoofExpected() { return s_spacewarSpoofExpected.load(std::memory_order_acquire); }

}  // namespace ac::presence
