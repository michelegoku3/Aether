#pragma once
#include <filesystem>
#include <fstream>
#include <string>
#include "utils/Strings.h"
namespace ac::deskpaths {
// Resolve once at startup; config and backup consumers share the same path.
inline std::string ReadDataDir(const std::filesystem::path& coreDir) {
    std::ifstream file(coreDir / "desk_path.cfg", std::ios::binary);
    std::string line;
    if (!std::getline(file, line)) return {};
    if (line.compare(0, 3, "\xEF\xBB\xBF") == 0) line.erase(0, 3);
    line = strings::Trim(line);
    if (line.empty() || line.find('\0') != std::string::npos) return {};
    return line;
}
inline std::filesystem::path ConfigPath(const std::string& dataDir) {
    return std::filesystem::path(dataDir) / "config" / "aethercore.toml";
}
}  // namespace ac::deskpaths
