#pragma once

namespace ac {

// Copies steamclient64.dll to bin\acoverlay.dll and loads that copy, giving us
// a private, hookable instance independent of the live client. Populates
// g_state.diversionModule. Returns false on failure.
bool LoadDiversion();

}  // namespace ac
