#include "pch.h"
#include "utils/PatternStateParse.h"

#include <limits>

namespace ac::pattern {
namespace {

constexpr std::size_t kShaLength = 64;

bool IsHexDigit(char c) { return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'); }

char ToLower(char c) { return (c >= 'A' && c <= 'Z') ? static_cast<char>(c - 'A' + 'a') : c; }

std::string_view Trim(std::string_view text) {
    while (!text.empty() && (text.front() == ' ' || text.front() == '\t'))
        text.remove_prefix(1);
    while (!text.empty() && (text.back() == ' ' || text.back() == '\t'))
        text.remove_suffix(1);
    return text;
}

// An entry is exactly two tokens: the sha256 and the decimal generation.
bool SplitEntry(std::string_view line, std::string_view& outSha, std::string_view& outGeneration) {
    const std::size_t separator = line.find_first_of(" \t");
    if (separator == std::string_view::npos)
        return false;
    outSha = line.substr(0, separator);
    const std::string_view rest = Trim(line.substr(separator));
    if (rest.empty() || rest.find_first_of(" \t") != std::string_view::npos)
        return false;
    outGeneration = rest;
    return true;
}

// Decimal digits only: `+5`, `1_0` and an overflowing value are not a
// generation. 0 is rejected because it is the parser's own "unknown" value.
bool ParseGeneration(std::string_view text, std::uint64_t& out) {
    if (text.empty())
        return false;
    constexpr std::uint64_t kMax = std::numeric_limits<std::uint64_t>::max();
    std::uint64_t value = 0;
    for (const char c : text) {
        if (c < '0' || c > '9')
            return false;
        const auto digit = static_cast<std::uint64_t>(c - '0');
        if (value > (kMax - digit) / 10)
            return false;
        value = value * 10 + digit;
    }
    if (value == 0)
        return false;
    out = value;
    return true;
}

std::string Lowercase(std::string_view text) {
    std::string out(text);
    for (char& c : out)
        c = ToLower(c);
    return out;
}

} // namespace

StateParse ParseState(std::string_view body) {
    StateParse out;

    // A checkout that round-tripped through Windows may carry a UTF-8 BOM.
    if (body.size() >= 3 && body.substr(0, 3) == "\xEF\xBB\xBF")
        body.remove_prefix(3);

    std::string section; // empty: no section seen yet
    while (!body.empty()) {
        const std::size_t newline = body.find('\n');
        std::string_view line = body.substr(0, newline);
        body.remove_prefix(newline == std::string_view::npos ? body.size() : newline + 1);
        if (!line.empty() && line.back() == '\r')
            line.remove_suffix(1);
        line = Trim(line);
        if (line.empty() || line.front() == '#')
            continue;

        if (line.front() == '[') {
            // A broken header clears the section: the entries that follow it
            // must not be attributed to the module named above.
            if (line.size() < 3 || line.back() != ']') {
                ++out.rejected;
                section.clear();
                continue;
            }
            section = Lowercase(line.substr(1, line.size() - 2));
            continue;
        }

        std::string_view sha;
        std::string_view generation;
        std::uint64_t value = 0;
        bool hex = true;
        if (section.empty() || !SplitEntry(line, sha, generation) || sha.size() != kShaLength) {
            ++out.rejected;
            continue;
        }
        for (const char c : sha) {
            if (!IsHexDigit(c)) {
                hex = false;
                break;
            }
        }
        if (!hex || !ParseGeneration(generation, value)) {
            ++out.rejected;
            continue;
        }

        // The module map is created only for an entry that survives every
        // check, so a section of malformed lines leaves no empty section behind.
        auto [it, inserted] = out.modules[section].emplace(Lowercase(sha), value);
        if (!inserted) {
            ++out.rejected;
            if (value > it->second)
                it->second = value;
        }
    }
    return out;
}

std::uint64_t Lookup(const StateIndex& index, std::string_view module, std::string_view sha) {
    const auto moduleIt = index.find(Lowercase(module));
    if (moduleIt == index.end())
        return 0;
    const auto entry = moduleIt->second.find(Lowercase(sha));
    return entry == moduleIt->second.end() ? 0 : entry->second;
}

} // namespace ac::pattern
