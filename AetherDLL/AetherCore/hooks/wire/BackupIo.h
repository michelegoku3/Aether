#pragma once

#include <cstdint>
#include <string>

#include "core/SteamTypes.h"

// ============================================================================
// BackupIo — shared utilities of the backup subsystem (AetherData).
//
// Small infrastructural module used by every backup store: local time
// formatting, (per-process cached) AetherData path resolution and atomic
// writes of generated files.
// No dependency on the wire module: low coupling, high reuse.
// ============================================================================
namespace ac::backup::io {

// "1787052569" -> "18/08/2026 13:29:29" (local time).
std::string FormatUnixTime(std::uint64_t unixTime);

// "2026-08-22T10:57:16" (local time): generated-file header.
std::string FormatWallClockNow();

// AetherData path from <steam>\aethercore\desk_path.cfg, read ONCE per process
// (the file does not change while Steam is running). "" when absent.
std::string CachedDeskDataDir();

// <AetherData>\backup\<appid>\achievements (creates the tree). "" when backup
// is unavailable (no AetherDesk).
std::string BackupDirForApp(steam::AppId appId);

// <AetherData>\backup\playtime (creates the folder). "" when unavailable.
std::string BackupPlaytimeDir();

// Atomic replacement: tmp -> dst with MOVEFILE_REPLACE_EXISTING.
bool AtomicReplace(const std::string& tmp, const std::string& dst);

}  // namespace ac::backup::io
