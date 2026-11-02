#pragma once

#include <cstdint>

namespace ac {

// Attaches AetherCore to the live steamclient64.dll that Steam maps itself.
//
// The client is NEVER loaded here: mapping it before Steam does initializes it
// out of order and crashes steam.exe inside the C++ exception machinery
// (observed as an access violation in VCRUNTIME140.dll at startup). This waits
// for Steam's own mapping instead, up to timeoutMs.
//
// Returns false when the client does not appear within the budget, or when the
// only module of that name lives outside the Steam root: a hook there would run
// on a module the running Steam instance never calls.
bool AttachSteamclient(std::uint32_t timeoutMs);

} // namespace ac
