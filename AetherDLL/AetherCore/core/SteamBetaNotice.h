#pragma once

#include <string>

// ---------------------------------------------------------------------------
// Steam client beta notice.
//
// Aether supports the stable Steam client only. When the client runs a beta
// branch, one system box at Steam startup says so and how to switch back. For
// that session the beta box replaces the Hubcap key box: a player on an
// unsupported client must not be asked to fix their key first, and Steam
// startup must never stack two popups.
// ---------------------------------------------------------------------------
namespace ac::steambeta {

// Shows the one-shot notice on its own named worker, so Steam's startup thread
// is never blocked. An empty branch means the stable client: nothing to show.
// A second call in the same process is a no-op.
void StartNotice(const std::string& branch);

} // namespace ac::steambeta
