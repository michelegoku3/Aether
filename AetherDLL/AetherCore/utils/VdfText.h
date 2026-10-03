#pragma once

#include <cstddef>
#include <string>
#include <vector>

// ---------------------------------------------------------------------------
// Steam text VDF — shared line-level helpers (P11 unification).
//
// Steam's text KeyValues ("VDF": localconfig.vdf, libraryfolders.vdf, ...)
// are line-oriented: "key" ["value"], braces on their own (or trailing) line,
// backslashes escaped as `\\\\`. These helpers used to live in duplicate
// inside PlaytimeMirror and dllmain; this header is the single copy.
// Header-only and pure: no logging, no filesystem, no g_state.
// ---------------------------------------------------------------------------
namespace ac::vdf {

enum class LineKind { None, SectionKey, KeyValue };

// Parses one line: `"key" "value"` -> KeyValue; a lone `"key"` -> SectionKey
// (in Steam VDFs the opening brace sits on the NEXT line); anything else
// (braces, garbage) -> None.
inline LineKind ParseLine(const std::string& line, std::string& key, std::string& value) {
    std::size_t i = 0;
    const std::size_t n = line.size();
    auto skipWs = [&] { while (i < n && (line[i] == ' ' || line[i] == '\t')) ++i; };
    auto readQuoted = [&](std::string& out) {
        skipWs();
        if (i >= n || line[i] != '"') return false;
        ++i;
        out.clear();
        while (i < n && line[i] != '"') out += line[i++];
        if (i >= n) return false;   // unterminated quote
        ++i;
        return true;
    };
    if (!readQuoted(key)) return LineKind::None;
    skipWs();
    if (i >= n) return LineKind::SectionKey;          // "key" (brace follows)
    if (line[i] == '{' || line[i] == '}') return LineKind::None;
    if (!readQuoted(value)) return LineKind::None;
    return LineKind::KeyValue;
}

// First `{` or `}` outside double quotes; true + position when found.
inline bool FindUnquotedBrace(const std::string& line, std::size_t& pos, char& brace) {
    bool inQuote = false;
    for (std::size_t i = 0; i < line.size(); ++i) {
        if (line[i] == '"') inQuote = !inQuote;
        else if (!inQuote && (line[i] == '{' || line[i] == '}')) {
            pos = i;
            brace = line[i];
            return true;
        }
    }
    return false;
}

// VDF escapes Windows separators as `\\\\` -> turn them back into `\\`.
inline void UnescapeBackslashes(std::string& s) {
    std::string::size_type pos = 0;
    while ((pos = s.find("\\\\", pos)) != std::string::npos) {
        s.replace(pos, 2, "\\");
        ++pos;
    }
}

// Every `"value"` bound to `key` (case-insensitive) anywhere in the content,
// unescaped. Replaces ad-hoc regex scans (libraryfolders.vdf "path").
inline std::vector<std::string> ExtractQuotedValues(const std::string& content,
                                                     const std::string& key) {
    std::vector<std::string> out;
    std::size_t lineStart = 0;
    while (lineStart <= content.size()) {
        std::size_t lineEnd = content.find('\n', lineStart);
        if (lineEnd == std::string::npos) lineEnd = content.size();
        std::string line = content.substr(lineStart, lineEnd - lineStart);
        if (!line.empty() && line.back() == '\r') line.pop_back();
        std::string k, v;
        if (ParseLine(line, k, v) == LineKind::KeyValue) {
            bool same = k.size() == key.size();
            for (std::size_t i = 0; same && i < k.size(); ++i) {
                const char a = k[i], b = key[i];
                same = (a == b) ||
                       (a >= 'A' && a <= 'Z' && static_cast<char>(a - 'A' + 'a') == b) ||
                       (b >= 'A' && b <= 'Z' && static_cast<char>(b - 'A' + 'a') == a);
            }
            if (same) {
                UnescapeBackslashes(v);
                out.push_back(std::move(v));
            }
        }
        if (lineEnd == content.size()) break;
        lineStart = lineEnd + 1;
    }
    return out;
}

// Replaces the value of the FIRST `"key" "<value>"` line (case-insensitive
// key match, same line semantics as ExtractQuotedValues) with newValue,
// touching only the value span. Returns false when no such line exists —
// content is left unchanged. Line-aware on purpose: a naive "find the next
// three quotes" scan breaks when the pair is the last quoted field in the
// file (real appmanifest_480.acf case), because no third quote follows.
inline bool ReplaceFirstQuotedValue(std::string& content, const std::string& key,
                                    const std::string& newValue) {
    std::size_t lineStart = 0;
    while (lineStart <= content.size()) {
        std::size_t lineEnd = content.find('\n', lineStart);
        if (lineEnd == std::string::npos) lineEnd = content.size();
        std::size_t stop = lineEnd;
        if (stop > lineStart && content[stop - 1] == '\r') --stop;   // strip \r

        std::size_t i = lineStart;
        auto skipWs = [&] { while (i < stop && (content[i] == ' ' || content[i] == '\t')) ++i; };
        // Reads `"token"` starting at the first non-ws char; sets open/close
        // quote indices. Returns false when no complete quoted token exists.
        auto readQuoted = [&](std::size_t& open, std::size_t& close) {
            skipWs();
            if (i >= stop || content[i] != '"') return false;
            open = i++;
            while (i < stop && content[i] != '"') ++i;
            if (i >= stop) return false;      // unterminated quote
            close = i++;
            return true;
        };

        std::size_t kOpen = 0, kClose = 0, vOpen = 0, vClose = 0;
        if (readQuoted(kOpen, kClose)) {
            const std::string k = content.substr(kOpen + 1, kClose - kOpen - 1);
            bool same = k.size() == key.size();
            for (std::size_t j = 0; same && j < k.size(); ++j) {
                const char a = k[j], b = key[j];
                same = (a == b) ||
                       (a >= 'A' && a <= 'Z' && static_cast<char>(a - 'A' + 'a') == b) ||
                       (b >= 'A' && b <= 'Z' && static_cast<char>(b - 'A' + 'a') == a);
            }
            if (same && readQuoted(vOpen, vClose)) {
                content.replace(vOpen + 1, vClose - vOpen - 1, newValue);
                return true;
            }
        }

        if (lineEnd == content.size()) break;
        lineStart = lineEnd + 1;
    }
    return false;
}

}  // namespace ac::vdf
