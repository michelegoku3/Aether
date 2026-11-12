#pragma once

#include <cstdint>
#include <string>

namespace ac {

// Waits for Steam to map one of its own modules, never loads it.
//
// AetherCore runs inside steam.exe, so the modules it hooks are mapped by
// Steam itself; mapping them earlier initializes them out of boot order and
// crashes the host inside the C++ exception machinery (observed as an access
// violation in VCRUNTIME140.dll at startup). A module of the same name mapped
// from another directory is refused: hooks there would run on an instance the
// running Steam never calls.
//
// `outcome` is one of "mapped", "not-mapped" or "path-invalid". `logModule` is
// the logger tag (each caller reports under its own area).
HMODULE WaitForMappedModule(const char* baseName, const std::string& expectedPath, std::uint32_t timeoutMs,
                            const char* logModule, std::string& outcome);

} // namespace ac
