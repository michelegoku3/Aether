#pragma once

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

#include "utils/Strings.h"
#include "utils/VdfText.h"
#include "utils/WinFile.h"

// ---------------------------------------------------------------------------
// Steam library discovery.
//
// One place for "every steamapps directory on this machine": the primary
// install plus each library listed in steamapps/libraryfolders.vdf. Callers
// are filesystem jobs (DirWatch, ACF maintenance), so the helper returns
// UTF-8 paths and never throws.
//
// Header-only and dependency-light: no logging, no g_state.
// ---------------------------------------------------------------------------
namespace ac::steamlibrary {

inline std::vector<std::string> SteamAppsDirs(const std::string& steamInstallPath) {
    std::vector<std::string> dirs;
    if (steamInstallPath.empty())
        return dirs;

    const std::filesystem::path primary = winfile::ToPath(steamInstallPath) / "steamapps";
    dirs.push_back(ac::strings::Narrow(primary.wstring()));

    std::ifstream input(primary / "libraryfolders.vdf");
    if (!input.is_open())
        return dirs;

    const std::string content((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
    for (const std::string& library : vdf::ExtractQuotedValues(content, "path")) {
        const std::filesystem::path steamapps = winfile::ToPath(library) / "steamapps";
        const std::string narrow = ac::strings::Narrow(steamapps.wstring());
        if (!narrow.empty() && std::find(dirs.begin(), dirs.end(), narrow) == dirs.end())
            dirs.push_back(narrow);
    }
    return dirs;
}

} // namespace ac::steamlibrary
