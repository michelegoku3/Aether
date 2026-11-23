#include "pch.h"
#include "utils/PatternSidecar.h"

#include <fstream>
#include <limits>
#include <string_view>

#include "utils/WinFile.h"

namespace ac::pattern::sidecar {
namespace {

constexpr std::string_view kVersionPrefix = "version=";

std::string TrimTrailing(std::string text) {
    while (!text.empty() && (text.back() == '\r' || text.back() == '\n' || text.back() == ' ' || text.back() == '\t'))
        text.pop_back();
    return text;
}

// Decimal digits only, no sign, no separators: anything else is a corrupt
// sidecar and must not be believed as a generation.
std::uint64_t ParseVersion(const std::string& line) {
    if (line.rfind(kVersionPrefix, 0) != 0)
        return 0;
    const std::string digits = line.substr(kVersionPrefix.size());
    if (digits.empty())
        return 0;
    constexpr std::uint64_t kMax = std::numeric_limits<std::uint64_t>::max();
    std::uint64_t value = 0;
    for (const char c : digits) {
        if (c < '0' || c > '9')
            return 0;
        const auto digit = static_cast<std::uint64_t>(c - '0');
        if (value > (kMax - digit) / 10)
            return 0;
        value = value * 10 + digit;
    }
    return value;
}

} // namespace

Record Read(const std::string& path) {
    Record record;
    std::ifstream in(winfile::ToPath(path));
    if (!in)
        return record;

    std::string line;
    if (std::getline(in, line))
        record.source = TrimTrailing(line);
    if (std::getline(in, line))
        record.version = ParseVersion(TrimTrailing(line));
    return record;
}

bool Write(const std::string& path, const std::string& source, std::uint64_t version) {
    const std::string tmp = path + ".tmp";
    {
        std::ofstream out(winfile::ToPath(tmp), std::ios::binary | std::ios::trunc);
        if (!out.is_open())
            return false;
        out << source << "\nversion=" << version << "\n";
        out.flush();
        if (!out.good()) {
            out.close();
            winfile::Delete(tmp);
            return false;
        }
    }
    if (!winfile::MoveReplace(tmp, path)) {
        winfile::Delete(tmp);
        return false;
    }
    return true;
}

} // namespace ac::pattern::sidecar
