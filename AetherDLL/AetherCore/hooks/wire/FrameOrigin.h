#pragma once

#include <cstdint>
#include <vector>

#include "hooks/wire/PacketRouter.h"

// ---------------------------------------------------------------------------
// FrameOrigin — connection capture for client-originated frames.
//
// Remembers the connection object and the CMsgProtoBufHeader (steamid +
// client_sessionid) from real outbound traffic, so SendClientFrame can
// originate a frame on the same socket/session (used to ask the CM for
// AppInfo records the local cache lacks). Value snapshot out, never Steam
// pointers across threads.
//
// Two slots: the last frame of any kind, and the last *service call*. The CM
// only answers a composed call that carries an authenticated service identity,
// and the last frame can be a heartbeat with steamid/session 0 (typical right
// after startup). A call composed at startup (the achievement catalog prefetch)
// must come from the service slot.
// ---------------------------------------------------------------------------
namespace ac::hooks {

struct FrameOrigin {
    void* obj = nullptr;
    std::vector<std::uint8_t> header;
};

// Captures from an observed outbound frame (no-op when the header is empty
// or oversized). Thread-safe; called on Steam's network thread.
void CaptureOrigin(void* obj, const WireFrame& f);

// Same, for a frame that is a client service call: that header carries the
// identity the CM accepts.
void CaptureServiceOrigin(void* obj, const WireFrame& f);

// Value-copy snapshots for SendClientFrame. Never null: empty when nothing
// has been captured yet.
FrameOrigin SnapshotOrigin();
FrameOrigin SnapshotServiceOrigin();

} // namespace ac::hooks
