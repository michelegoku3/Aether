#pragma once

#include <algorithm>
#include <cctype>
#include <filesystem>
#include <string>
#include <string_view>

// ---------------------------------------------------------------------------
// Shared path/string micro-helpers (quick-win batch 2026-10-03).
//
// These five little helpers used to be copy-pasted across PipeWatch,
// OwnershipHooks, DirWatch, ManifestRestore and LuaBindings. One copy lives
// here; header-only, no state, no logging.
// ---------------------------------------------------------------------------
namespace ac::paths {

// ASCII lowercase copy (locale-independent: Steam identifiers are ASCII).
inline std::string LowerAscii(std::string_view text) {
    std::string out(text);
    std::transform(out.begin(), out.end(), out.begin(), [](unsigned char ch) {
        return static_cast<char>(std::tolower(ch));
    });
    return out;
}

// File name component after the last separator ('\\' or '/'); the whole text
// when there is none.
inline std::string BaseName(std::string_view path) {
    const std::size_t slash = path.find_last_of("\\/");
    if (slash == std::string_view::npos) return std::string(path);
    return std::string(path.substr(slash + 1));
}

// Case-insensitive extension match (a leading dot is part of `wanted`).
inline bool HasExtension(const std::filesystem::path& path, const char* wanted) {
    std::string ext = path.extension().string();
    std::string expected = wanted;
    if (ext.size() != expected.size()) return false;
    for (char& c : ext) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    for (char& c : expected) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return ext == expected;
}

}  // namespace ac::paths
