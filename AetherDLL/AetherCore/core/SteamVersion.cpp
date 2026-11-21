#include "pch.h"
#include "core/SteamVersion.h"

#include <cstdint>
#include <filesystem>
#include <fstream>

#include "core/Logger.h"
#include "utils/WinFile.h"

namespace ac {
namespace {
constexpr const char* kModule = "SteamVersion";
// A branch name is a handful of bytes ("publicbeta"); a bigger file is not the
// channel marker and must not be read into the branch string.
constexpr std::uintmax_t kMaxBranchBytes = 64;
} // namespace

std::string DetectSteamBuildId() {
    using GetBootstrapperVersion_t = std::int64_t (*)();

    HMODULE steam = GetModuleHandleA("steam.exe");
    if (!steam) {
        AC_LOG_WARN(kModule, "steam.exe not loaded; build id unavailable.");
        return {};
    }

    auto fn = reinterpret_cast<GetBootstrapperVersion_t>(GetProcAddress(steam, "GetBootstrapperVersion"));
    if (!fn) {
        AC_LOG_WARN(kModule, "GetBootstrapperVersion not exported; build id unavailable.");
        return {};
    }

    std::string buildId = std::to_string(fn());
    AC_LOG_INFO(kModule, "Steam build id = %s", buildId.c_str());
    return buildId;
}

std::string DetectSteamBetaBranch(const std::string& steamInstallPath) {
    if (steamInstallPath.empty())
        return {};

    // The install path is UTF-8; building the path from a narrow string would
    // decode it via the ANSI codepage and miss non-ASCII Steam roots entirely.
    const std::filesystem::path marker = winfile::ToPath(steamInstallPath) / "package" / "beta";
    std::error_code ec;
    const std::uintmax_t size = std::filesystem::file_size(marker, ec);
    if (ec) {
        AC_LOG_DEBUG(kModule, "Steam client channel: stable (no package/beta marker).");
        return {};
    }
    if (size == 0) {
        AC_LOG_DEBUG(kModule, "Steam client channel: stable (empty package/beta marker).");
        return {};
    }
    if (size > kMaxBranchBytes) {
        AC_LOG_WARN(kModule, "Ignoring implausible package/beta marker (%llu bytes).",
                    static_cast<unsigned long long>(size));
        return {};
    }

    std::ifstream file(marker, std::ios::binary);
    if (!file) {
        AC_LOG_DEBUG(kModule, "Steam client channel: stable (package/beta unreadable).");
        return {};
    }
    char buffer[kMaxBranchBytes];
    file.read(buffer, sizeof(buffer));
    std::string branch = ParseBetaBranch(std::string_view(buffer, static_cast<std::size_t>(file.gcount())));
    if (branch.empty()) {
        AC_LOG_DEBUG(kModule, "Steam client channel: stable (package/beta is not a branch name).");
        return {};
    }
    AC_LOG_INFO(kModule, "Steam client channel: %s (beta).", branch.c_str());
    return branch;
}

} // namespace ac
