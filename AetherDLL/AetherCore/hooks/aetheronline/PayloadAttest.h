#pragma once

#include <windows.h>

#include <string>

namespace ac::hooks::payloadattest {

// Same release when both version words match. The payload and the core are
// built from the same PROJECT_VERSION, so any difference marks a foreign file.
inline bool VersionsMatch(DWORD selfMs, DWORD selfLs, DWORD payloadMs, DWORD payloadLs) {
    return selfMs == payloadMs && selfLs == payloadLs;
}

// Reads the VS_FIXEDFILEINFO of this module and of `payloadPath` and compares
// them. False when either file has no version resource or the versions differ.
bool PayloadVersionMatches(HMODULE selfModule, const std::string& payloadPath);

} // namespace ac::hooks::payloadattest
