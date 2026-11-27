#pragma once

#include <cstdint>
#include <vector>

#include "hooks/wire/PacketRouter.h"

// FrameOrigin — authenticated CM transport capture for client-originated frames.
//
// Steam sends through CNet (carrying a uint32_t hConnection across UDP, TCP, and
// WebSocket) or through CWebSocketConnection fallback (carrying a WebSocket obj
// pointer). Both paths must only record frames carrying a non-zero authenticated
// steamid so local WebUI / pre-login frames never overwrite the active CM identity.
namespace ac::hooks {

struct FrameOrigin {
    std::uint32_t hConnection = 0;
    void* obj = nullptr;
    std::vector<std::uint8_t> header;
};

void CaptureOrigin(std::uint32_t hConnection, void* obj, const WireFrame& f);
void CaptureServiceOrigin(std::uint32_t hConnection, void* obj, const WireFrame& f);
FrameOrigin SnapshotOrigin();
FrameOrigin SnapshotServiceOrigin();

} // namespace ac::hooks
