#include "pch.h"
#include "hooks/aetheronline/PayloadAttest.h"

#include <cstdint>
#include <vector>

#include "utils/Strings.h"

#pragma comment(lib, "version.lib")

namespace ac::hooks::payloadattest {
namespace {

// Reads the VS_FIXEDFILEINFO version of a DLL. False when the file has no
// version resource (never in real builds: verify_version.ps1 hard-gates it).
bool GetFileVersion(const std::string& utf8Path, DWORD& ms, DWORD& ls) {
    // UTF-8 in, *W at the boundary: version APIs have no UTF-8 entry point.
    const std::wstring wide = ac::strings::Widen(utf8Path);
    if (wide.empty())
        return false;
    DWORD handle = 0;
    const DWORD size = GetFileVersionInfoSizeW(wide.c_str(), &handle);
    if (size == 0)
        return false;
    std::vector<std::uint8_t> data(size);
    if (!GetFileVersionInfoW(wide.c_str(), 0, size, data.data()))
        return false;
    void* block = nullptr;
    UINT len = 0;
    if (!VerQueryValueW(data.data(), L"\\", &block, &len) || !block || len < sizeof(VS_FIXEDFILEINFO))
        return false;
    const auto* info = static_cast<const VS_FIXEDFILEINFO*>(block);
    if (info->dwSignature != 0xFEEF04BD)
        return false;
    ms = info->dwFileVersionMS;
    ls = info->dwFileVersionLS;
    return true;
}

} // namespace

bool PayloadVersionMatches(HMODULE selfModule, const std::string& payloadPath) {
    if (!selfModule)
        return false;
    // Wide at the boundary (see Diversion.cpp): *A would mangle non-ASCII
    // Steam installs via the system codepage.
    DWORD selfMs = 0, selfLs = 0;
    std::wstring selfPath(32768, L'\0');
    const DWORD written = GetModuleFileNameW(selfModule, selfPath.data(), static_cast<DWORD>(selfPath.size()));
    if (written == 0 || written >= selfPath.size())
        return false;
    const std::string narrowSelf = ac::strings::Narrow(selfPath);
    if (narrowSelf.empty() || !GetFileVersion(narrowSelf, selfMs, selfLs))
        return false;

    DWORD payloadMs = 0, payloadLs = 0;
    if (!GetFileVersion(payloadPath, payloadMs, payloadLs))
        return false;
    return VersionsMatch(selfMs, selfLs, payloadMs, payloadLs);
}

} // namespace ac::hooks::payloadattest
