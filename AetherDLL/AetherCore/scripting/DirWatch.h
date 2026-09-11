#pragma once

#include <string>
#include <vector>

// ---------------------------------------------------------------------------
// Directory watcher.
//
// Watches the configured stplug-in directories for Lua hot reloads and
// Steam\steamapps for appmanifest_<app_id>.acf removals. An ACF removal is
// treated as a completed uninstall signal and triggers a targeted manifest
// restore from AetherData\backup\<app_id>\lua into Steam\depotcache.
// ---------------------------------------------------------------------------
namespace ac::dirwatch {

// Starts the Lua watcher. No-op if already running or both lists are empty.
void Start(const std::vector<std::string>& directories);

// Starts the Lua watcher plus a non-recursive watcher for Steam app manifests.
void Start(const std::vector<std::string>& directories,
           const std::vector<std::string>& acfDirectories);

// Signals the thread to stop and joins it. Safe to call if never started.
void Stop();

}  // namespace ac::dirwatch
