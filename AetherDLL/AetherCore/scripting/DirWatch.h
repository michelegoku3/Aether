#pragma once

#include <string>
#include <vector>

// ---------------------------------------------------------------------------
// Lua hot-reload directory watcher.
//
// Watches the configured stplug-in directories with ReadDirectoryChangesW on a
// dedicated thread. On a debounced batch of .lua changes it re-parses / unloads
// the affected files and triggers a single license refresh, so games appear and
// disappear without restarting Steam.
// ---------------------------------------------------------------------------
namespace ac::dirwatch {

// Starts the watcher thread for the given directories. No-op if already running
// or the list is empty.
void Start(const std::vector<std::string>& directories);

// Signals the thread to stop and joins it. Safe to call if never started.
void Stop();

}  // namespace ac::dirwatch
