#pragma once

#include <string>

// ---------------------------------------------------------------------------
// SteamUIStylePatch — purely visual tweaks to the Steam client stylesheet.
//
// The library app-details page renders its "legacy library sharing" notice as
// a row carrying the CSS-module class mapped to `OldSharingWarning` (see
// steamui/sp.js), styled with `color:orange`. That row is the orange
// deprecation text shown next to the "From the library of <account>" banner;
// it is not part of the banner itself and carries no state we need.
//
// The tweak rewrites that rule IN PLACE with same-length swaps, so the
// stylesheet's size never changes. That matters: Steam's bootstrapper verifies
// client file sizes before every start and re-applies the whole client package
// on any mismatch, which shows up as a bogus "Updating Steam" screen.
//
// Hiding it is a stylesheet-only change: the class name is content-hashed per
// Steam build, so it is resolved from sp.js at runtime. Turning the feature off
// rewrites the rule back to its original bytes. Both directions are idempotent
// and re-applied on the next start; a Steam build whose rule no longer matches
// is reported and the stylesheet is left untouched.
//
// No JS is modified and no behavior changes beyond the hidden row.
// ---------------------------------------------------------------------------
namespace ac::steamui {

// Applies the stylesheet policy once per process, before Steam's UI loads:
//   hide = true  -> hide the legacy sharing warning;
//   hide = false -> restore the original rule.
// Best-effort and idempotent.
void ApplyLegacySharingWarningPolicy(const std::string& steamInstallPath, bool hide);

} // namespace ac::steamui
