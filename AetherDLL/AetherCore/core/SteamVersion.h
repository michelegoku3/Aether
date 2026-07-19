#pragma once

#include <string>

namespace ac {

// Reads the Steam build number via steam.exe!GetBootstrapperVersion. Returns
// the number as a string, or empty if steam.exe is not loaded / does not export
// the function. Diagnostic only — never fatal.
std::string DetectSteamBuildId();

}  // namespace ac
