#include "pch.h"
#include "core/SteamVersion.h"

#include <cstdint>

#include "core/Logger.h"

namespace ac {
namespace {
constexpr const char* kModule = "SteamVersion";
}

std::string DetectSteamBuildId() {
    using GetBootstrapperVersion_t = std::int64_t (*)();

    HMODULE steam = GetModuleHandleA("steam.exe");
    if (!steam) {
        AC_LOG_WARN(kModule, "steam.exe not loaded; build id unavailable.");
        return {};
    }

    auto fn = reinterpret_cast<GetBootstrapperVersion_t>(
        GetProcAddress(steam, "GetBootstrapperVersion"));
    if (!fn) {
        AC_LOG_WARN(kModule, "GetBootstrapperVersion not exported; build id unavailable.");
        return {};
    }

    std::string buildId = std::to_string(fn());
    AC_LOG_INFO(kModule, "Steam build id = %s", buildId.c_str());
    return buildId;
}

}  // namespace ac
