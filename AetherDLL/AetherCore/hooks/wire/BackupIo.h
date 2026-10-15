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

// Unique per-process staging path for atomic copies sharing one directory.
// Two writers (e.g. AetherDesk and this DLL backing up the same manifest)
// must never share a staging name: the loser of a failure cleanup would
// delete the winner's in-flight file. The counter is process-wide and
// atomic, hence unique across threads; staying in the destination directory
// keeps the final rename on the same volume (atomic).
// Defined in BackupIo.cpp (needs the OS process id; the header stays free
// of <windows.h> so test doubles with fake Win32 types keep compiling).
std::string UniqueStagingPath(const std::string& destination);

// Size verification with distinct source/destination errors. A single shared
// error_code would let a source stat failure mask the destination result;
// callers log srcError/dstError separately so the failing side is named.
struct SizeCheck {
    bool ok = false;
    std::string srcError;
    std::string dstError;
};

SizeCheck VerifySameSize(const std::string& source, const std::string& destination);

} // namespace ac::backup::io
