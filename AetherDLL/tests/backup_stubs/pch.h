#pragma once
// Filesystem enumeration seam: lifecycle tests never read the host's Steam data.
#include <cstdint>
#include <cstring>
using HMODULE = void*;
using DWORD = unsigned long;
using HANDLE = void*;
inline HANDLE const INVALID_HANDLE_VALUE = reinterpret_cast<HANDLE>(std::intptr_t{-1});
struct WIN32_FIND_DATAA { char cFileName[260]{}; };
namespace backup_test { inline bool scanFile = false; }
inline HANDLE FindFirstFileA(const char*, WIN32_FIND_DATAA* data) {
    if (!backup_test::scanFile) return INVALID_HANDLE_VALUE;
    std::strcpy(data->cFileName, "UserGameStats_1234567890_1234567890.bin");
    return reinterpret_cast<HANDLE>(std::intptr_t{1});
}
inline bool FindNextFileA(HANDLE, WIN32_FIND_DATAA*) { return false; }
inline void FindClose(HANDLE) {}
