#pragma once

#include <array>
#include <string_view>

// ---------------------------------------------------------------------------
// Build-independent fallback patterns.
//
// These entries are intentionally kept in one small, reviewable file because
// the remote TOML source does not currently publish every pattern required by
// AetherCore. PatternEngine merges them with TOML entries at load time:
//   * TOML wins when it contains the same function name;
//   * this table fills only missing entries;
//   * every entry is still bounds-checked, and signatures are verified when
//     supplied.
//
// Add future fallback entries here, not in hook modules. Keep the current
// Steam build/version and the evidence for every RVA in the commit message or
// an adjacent comment. Prefer a complete signature; an RVA-only entry is a
// compatibility fallback and is logged as such.
// ---------------------------------------------------------------------------
namespace ac::pattern {

struct HardcodedPattern {
    std::string_view module;
    std::string_view name;
    std::string_view rva;
    std::string_view sig; // empty = legacy RVA-only fallback
};

inline constexpr std::array<HardcodedPattern, 5> kHardcodedPatterns = {{
    // Current fallbacks formerly embedded in LicenseHooks.cpp.
    {"steamclient", "OptedInMask",                "0x5DD630", ""},
    {"steamclient", "RequiresLegacyCDKey",       "0x83C490", ""},
    {"steamclient", "IsCloudEnabledForApp",      "0x8217E0",
        "40 53 56 57 48 83 EC ?? 8B DA 48 8B F9 BA 40 00 00 00 48 8D 4C 24 30 45 33 C9 41 B8 20 00 00 00 E8 ?? ?? ?? ?? B2 01"},
    {"steamclient", "GetRemoteStorageSyncState", "0x775880",
        "40 53 56 57 48 83 EC ?? 8B DA 48 8B F9 BA 40 00 00 00 48 8D 4C 24 30 45 33 C9 41 B8 20 00 00 00 E8 ?? ?? ?? ??"},
    {"steamclient", "CloseAppCloud",              "0xA1E450",
        "48 89 5C 24 10 57 48 83 EC 30 8B FA 48 8B D9 85 D2"},
}};

}  // namespace ac::pattern
