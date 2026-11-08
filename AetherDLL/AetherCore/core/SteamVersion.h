#pragma once

#include <string>
#include <string_view>

namespace ac {

// Reads the Steam build number via steam.exe!GetBootstrapperVersion. Returns
// the number as a string, or empty if steam.exe is not loaded / does not export
// the function. Diagnostic only — never fatal.
std::string DetectSteamBuildId();

// Steam records the selected client branch in <steam>\package\beta (the file
// holds the branch name, e.g. "publicbeta"); on the stable client it is absent
// or empty. That file is the only reliable channel signal: the registry keeps no
// BetaKey, and the packet layout is only known once traffic starts.
//
// Returns the trimmed branch only when the marker holds a plausible branch name
// (ASCII letters, digits, '.', '_', '-'), otherwise empty: a truncated or
// foreign file must not raise the notice. The name reaches the log, so control
// characters are rejected as well.
inline std::string ParseBetaBranch(std::string_view raw) {
    const auto isSpace = [](char c) {
        return c == ' ' || c == '\t' || c == '\r' || c == '\n' || c == '\f' || c == '\v';
    };
    const auto trim = [isSpace](std::string_view value) {
        while (!value.empty() && isSpace(value.front()))
            value.remove_prefix(1);
        while (!value.empty() && isSpace(value.back()))
            value.remove_suffix(1);
        return value;
    };

    std::string_view branch = trim(raw);
    // The BOM may follow leading whitespace, so trim again after removing it.
    constexpr std::string_view kBom = "\xEF\xBB\xBF";
    if (branch.substr(0, kBom.size()) == kBom)
        branch = trim(branch.substr(kBom.size()));
    if (branch.empty())
        return {};

    for (const char c : branch) {
        const bool isNameChar = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') ||
                                c == '.' || c == '_' || c == '-';
        if (!isNameChar)
            return {};
    }
    return std::string(branch);
}

// Reads <steamInstallPath>\package\beta and returns its branch name, or empty on
// the stable client. Input for the startup notice; never fatal.
std::string DetectSteamBetaBranch(const std::string& steamInstallPath);

} // namespace ac
