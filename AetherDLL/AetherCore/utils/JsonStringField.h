#pragma once

#include <cstdint>
#include <string>
#include <string_view>

// ---------------------------------------------------------------------------
// Minimal scalar field pulls from a JSON object body.
//
// Not a JSON parser: no escapes, no nested objects, no floats. Good enough
// for the controlled payloads we read (eticket mint, shared quota state,
// generation usage). Prefer this over a full JSON dependency (zero bloat).
// ---------------------------------------------------------------------------
namespace ac::jsonutil {

inline bool PullStringField(std::string_view json, std::string_view key, std::string& out) {
    const std::string pattern = "\"" + std::string(key) + "\"";
    const std::size_t pos = json.find(pattern);
    if (pos == std::string_view::npos) return false;
    const std::size_t colon = json.find(':', pos + pattern.size());
    if (colon == std::string_view::npos) return false;
    const std::size_t q1 = json.find('"', colon + 1);
    if (q1 == std::string_view::npos) return false;
    const std::size_t q2 = json.find('"', q1 + 1);
    if (q2 == std::string_view::npos) return false;
    out.assign(json.data() + q1 + 1, q2 - q1 - 1);
    return !out.empty();
}

inline void SkipSpaces(std::string_view json, std::size_t& cursor) {
    while (cursor < json.size()) {
        const char c = json[cursor];
        if (c != ' ' && c != '\t' && c != '\r' && c != '\n') break;
        ++cursor;
    }
}

// Minimal "key":<digits> scalar pull. Digits only: no sign, float, or
// exponent (none of the fields we read ever carry one).
inline bool PullUIntField(std::string_view json, std::string_view key, std::uint64_t& out) {
    const std::string pattern = "\"" + std::string(key) + "\"";
    const std::size_t pos = json.find(pattern);
    if (pos == std::string_view::npos) return false;
    const std::size_t colon = json.find(':', pos + pattern.size());
    if (colon == std::string_view::npos) return false;
    std::size_t cursor = colon + 1;
    SkipSpaces(json, cursor);
    if (cursor >= json.size() || json[cursor] < '0' || json[cursor] > '9') return false;
    std::uint64_t value = 0;
    while (cursor < json.size() && json[cursor] >= '0' && json[cursor] <= '9') {
        value = value * 10 + static_cast<std::uint64_t>(json[cursor] - '0');
        ++cursor;
    }
    out = value;
    return true;
}

// Minimal "key":true|false scalar pull.
inline bool PullBoolField(std::string_view json, std::string_view key, bool& out) {
    const std::string pattern = "\"" + std::string(key) + "\"";
    const std::size_t pos = json.find(pattern);
    if (pos == std::string_view::npos) return false;
    const std::size_t colon = json.find(':', pos + pattern.size());
    if (colon == std::string_view::npos) return false;
    std::size_t cursor = colon + 1;
    SkipSpaces(json, cursor);
    const std::size_t remaining = json.size() - cursor;
    const auto starts_with = [&](std::string_view literal) {
        return remaining >= literal.size() && json.substr(cursor, literal.size()) == literal;
    };
    if (starts_with("true")) {
        out = true;
        return true;
    }
    if (starts_with("false")) {
        out = false;
        return true;
    }
    return false;
}

}  // namespace ac::jsonutil
