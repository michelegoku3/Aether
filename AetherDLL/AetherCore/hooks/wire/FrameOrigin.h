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
// ---------------------------------------------------------------------------
namespace ac::hooks {

struct FrameOrigin {
    void* obj = nullptr;
    std::vector<std::uint8_t> header;
};

// Captures from an observed outbound frame (no-op when the header is empty
// or oversized). Thread-safe; called on Steam's network thread.
void CaptureOrigin(void* obj, const WireFrame& f);

// Value-copy snapshot for SendClientFrame. Never null: empty when nothing
// has been captured yet.
FrameOrigin SnapshotOrigin();

} // namespace ac::hooks
