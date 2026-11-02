#pragma once

namespace ac {

// Attaches AetherCore to the live steamclient64.dll, the module Steam itself
// uses. There is no copy and no redirect: every hook must run on the client the
// running Steam instance actually calls.
//
// Returns false when the client cannot be mapped, or when the mapped module is
// not the one at the Steam root. The caller must abort in that case: hooks
// registered against a missing module would silently do nothing.
bool AttachSteamclient();

} // namespace ac
