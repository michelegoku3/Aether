#pragma once

#include <cstdint>

namespace ac {

// Attaches the live steamui.dll Steam maps inside steam.exe. Same contract as
// AttachSteamclient: the module is never loaded here, and only the copy at the
// Steam root is accepted.
//
// steamui can map later than the client, so the boot path calls this with a
// zero budget and lets a worker wait; a non-zero budget blocks the caller.
bool AttachSteamui(std::uint32_t timeoutMs);

} // namespace ac
