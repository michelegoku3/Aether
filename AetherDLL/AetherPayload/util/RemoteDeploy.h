#pragma once

#include <windows.h>

namespace ac::remoteinject {

/**
 * Loads a DLL into a remote process using CreateRemoteThread + LoadLibraryW.
 *
 * @param process   Handle to the target process (must have appropriate rights)
 * @param dllPath   Full path to the DLL to inject (must be accessible by target)
 * @return          true on success, false on failure
 */
bool LoadDll(HANDLE process, LPCWSTR dllPath);

}  // namespace ac::remoteinject