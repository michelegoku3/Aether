#pragma once

#include <string>
#include <string_view>

// ---------------------------------------------------------------------------
// Minimal "key":"value" string field pull from a JSON object body.
//
// Not a JSON parser: no escapes, no nested objects, no numbers. Good enough
// for the eticket mint backend response we control. Prefer this over a full
// JSON dependency (zero bloat).
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

}  // namespace ac::jsonutil
