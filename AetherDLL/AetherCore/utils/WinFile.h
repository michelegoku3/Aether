#pragma once

#include <filesystem>
#include <string>
#include <windows.h>

#include "utils/Strings.h"

// ---------------------------------------------------------------------------
// WinFile — UTF-8 filesystem primitives for Win32.
//
// Internal plumbing is UTF-8 narrow; Win32 takes *W at the boundary (*A
// would mangle non-ASCII paths via the system codepage). Empty/invalid
// input fails closed (false), never touches the filesystem.
// ---------------------------------------------------------------------------
namespace ac::winfile {

inline std::wstring WidenPath(const std::string& utf8) { return ac::strings::Widen(utf8); }

// Builds a native path from a UTF-8 string. std::filesystem::path from a
// narrow string would decode via the ANSI codepage; from wide it is exact.
// Use at every point where a runtime string becomes a path (g_state roots,
// config values, test/user input). Literals need nothing (pure ASCII).
inline std::filesystem::path ToPath(const std::string& utf8) { return std::filesystem::path(WidenPath(utf8)); }

inline bool Exists(const std::string& utf8Path) {
    const std::wstring wide = WidenPath(utf8Path);
    if (wide.empty())
        return false;
    return GetFileAttributesW(wide.c_str()) != INVALID_FILE_ATTRIBUTES;
}

inline bool IsDirectory(const std::string& utf8Path) {
    const std::wstring wide = WidenPath(utf8Path);
    if (wide.empty())
        return false;
    const DWORD attrs = GetFileAttributesW(wide.c_str());
    return attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_DIRECTORY) != 0;
}

// Creates one directory level; ERROR_ALREADY_EXISTS counts as success.
inline bool CreateDir(const std::string& utf8Path) {
    const std::wstring wide = WidenPath(utf8Path);
    if (wide.empty())
        return false;
    return CreateDirectoryW(wide.c_str(), nullptr) != FALSE || GetLastError() == ERROR_ALREADY_EXISTS;
}

// Native-path overload: no round-trip through narrow (which would decode
// via the ANSI codepage). Prefer this when a std::filesystem::path is
// already at hand.
inline bool CreateDirPath(const std::filesystem::path& dir) {
    if (dir.empty())
        return false;
    return CreateDirectoryW(dir.c_str(), nullptr) != FALSE || GetLastError() == ERROR_ALREADY_EXISTS;
}

inline bool MoveReplace(const std::string& fromUtf8, const std::string& toUtf8) {
    const std::wstring from = WidenPath(fromUtf8);
    const std::wstring to = WidenPath(toUtf8);
    if (from.empty() || to.empty())
        return false;
    return MoveFileExW(from.c_str(), to.c_str(), MOVEFILE_REPLACE_EXISTING) != FALSE;
}

inline bool Delete(const std::string& utf8Path) {
    const std::wstring wide = WidenPath(utf8Path);
    if (wide.empty())
        return false;
    return DeleteFileW(wide.c_str()) != FALSE;
}

inline bool CopyOverwrite(const std::string& fromUtf8, const std::string& toUtf8) {
    const std::wstring from = WidenPath(fromUtf8);
    const std::wstring to = WidenPath(toUtf8);
    if (from.empty() || to.empty())
        return false;
    return CopyFileW(from.c_str(), to.c_str(), FALSE) != FALSE;
}

} // namespace ac::winfile
