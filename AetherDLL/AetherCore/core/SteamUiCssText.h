#pragma once

#include <cstddef>
#include <string>
#include <string_view>

// ---------------------------------------------------------------------------
// Steam UI stylesheet editing (pure, header-only).
//
// The library app-details page renders the legacy library-sharing notice with
// the CSS-module class mapped to `OldSharingWarning` in steamui/sp.js; the row
// carries `color:orange`. Hiding it must NOT change the stylesheet's size:
// Steam's bootstrapper verifies client file SIZES before every start ("Verifying
// file sizes only") and re-applies the whole client package on any mismatch,
// which surfaces as a bogus "Updating Steam" screen.
//
// Both directions are therefore same-length swaps inside the rule body:
//   hide:    display:flex -> display:none   (the row stops being laid out)
//            color:orange -> /*aeth:hid*/   (marker + the orange accent goes)
//   restore: the exact reverse.
//
// Matches are declaration-aware: `color:orange` is only accepted as a whole
// declaration, so `background-color:orange` (which contains the same bytes)
// can never be selected and corrupted.
//
// The marker makes the operation self-describing and idempotent: a body that
// already carries it is left alone. Anything unexpected (a Steam build whose
// rule no longer matches) is reported as NotApplicable and the file is never
// touched.
// ---------------------------------------------------------------------------
namespace ac::steamui::css {

enum class WarningPatch {
    Changed,       // the rule was rewritten (hidden or restored)
    Unchanged,     // already in the requested state: no write needed
    NotApplicable, // the expected rule is absent: leave the sheet untouched
};

inline constexpr std::string_view kHideMarker = "/*aeth:hid*/";
inline constexpr std::string_view kOriginalDisplay = "display:flex";
inline constexpr std::string_view kHiddenDisplay = "display:none";
inline constexpr std::string_view kOriginalColor = "color:orange";

// Marker block written by the first revision of the patch (before the
// size-preserving rewrite). It changed the file size, so it is stripped on
// sight even when the rule itself can no longer be patched.
inline constexpr std::string_view kLegacyBegin = "/* aether:hide-old-sharing:begin */";
inline constexpr std::string_view kLegacyEnd = "/* aether:hide-old-sharing:end */";

static_assert(kOriginalDisplay.size() == kHiddenDisplay.size(),
              "hide/restore must preserve the file size: display swap differs in length");
static_assert(kOriginalColor.size() == kHideMarker.size(),
              "hide/restore must preserve the file size: marker differs in length");

inline bool IsDeclarationSpace(char c) { return c == ' ' || c == '\t' || c == '\r' || c == '\n' || c == '\f'; }

// A declaration starts after `;` (or at the body start) and ends at `;`, at the
// body end, or before `!important`.
inline bool IsDeclarationStartBoundary(char c) { return c == ';' || IsDeclarationSpace(c); }
inline bool IsDeclarationEndBoundary(char c) { return c == ';' || c == '!' || IsDeclarationSpace(c); }

// Index of the first whole-declaration occurrence of `declaration`, or npos.
// `background-color:orange` does not match a search for `color:orange` because
// the preceding byte is `-`, not a declaration boundary.
inline std::size_t FindDeclaration(std::string_view body, std::string_view declaration) {
    if (declaration.empty())
        return std::string_view::npos;
    for (std::size_t at = body.find(declaration); at != std::string_view::npos; at = body.find(declaration, at + 1)) {
        const bool startOk = at == 0 || IsDeclarationStartBoundary(body[at - 1]);
        const std::size_t after = at + declaration.size();
        const bool endOk = after >= body.size() || IsDeclarationEndBoundary(body[after]);
        if (startOk && endOk)
            return at;
    }
    return std::string_view::npos;
}

// Number of whole-declaration occurrences of `declaration`.
inline std::size_t CountDeclarations(std::string_view body, std::string_view declaration) {
    if (declaration.empty())
        return 0;
    std::size_t count = 0;
    std::size_t offset = 0;
    while (offset < body.size()) {
        const std::size_t at = FindDeclaration(body.substr(offset), declaration);
        if (at == std::string_view::npos)
            break;
        ++count;
        offset += at + declaration.size();
    }
    return count;
}

// Body [begin, end) of the first `.<cssClass>{...}` rule that carries either
// the orange accent or our marker. Other rules with the same class (the layout
// variants) are skipped. Returns false when no such rule exists.
inline bool FindWarningBody(std::string_view css, std::string_view cssClass, std::size_t& begin, std::size_t& end) {
    if (cssClass.empty())
        return false;
    const std::string selector = "." + std::string(cssClass);
    for (std::size_t at = css.find(selector); at != std::string_view::npos; at = css.find(selector, at + 1)) {
        const std::size_t brace = at + selector.size();
        if (brace >= css.size() || css[brace] != '{')
            continue;
        const std::size_t bodyEnd = css.find('}', brace + 1);
        if (bodyEnd == std::string_view::npos)
            continue;
        const std::string_view body = css.substr(brace + 1, bodyEnd - brace - 1);
        if (FindDeclaration(body, kOriginalColor) != std::string_view::npos ||
            FindDeclaration(body, kHideMarker) != std::string_view::npos) {
            begin = brace + 1;
            end = bodyEnd;
            return true;
        }
    }
    return false;
}

// Hides (hide = true) or restores (hide = false) the legacy sharing notice.
// Never changes `css`'s size.
inline WarningPatch ApplyWarningPatch(std::string& css, std::string_view cssClass, bool hide) {
    std::size_t begin = 0;
    std::size_t end = 0;
    if (!FindWarningBody(css, cssClass, begin, end))
        return WarningPatch::NotApplicable;

    const std::string_view body = std::string_view(css).substr(begin, end - begin);
    const bool hidden = FindDeclaration(body, kHideMarker) != std::string_view::npos;
    if (hide == hidden)
        return WarningPatch::Unchanged;

    const std::string_view fromDisplay = hide ? kOriginalDisplay : kHiddenDisplay;
    const std::string_view toDisplay = hide ? kHiddenDisplay : kOriginalDisplay;
    const std::string_view fromColor = hide ? kOriginalColor : kHideMarker;
    const std::string_view toColor = hide ? kHideMarker : kOriginalColor;

    // Exactly one of each: a body with duplicates is not the rule we understand.
    if (CountDeclarations(body, fromDisplay) != 1 || CountDeclarations(body, fromColor) != 1)
        return WarningPatch::NotApplicable;

    const std::size_t displayAt = FindDeclaration(body, fromDisplay);
    const std::size_t colorAt = FindDeclaration(body, fromColor);
    css.replace(begin + displayAt, fromDisplay.size(), toDisplay);
    css.replace(begin + colorAt, fromColor.size(), toColor);
    return WarningPatch::Changed;
}

// Removes the old marker block, plus the newline that introduced it. Returns
// true when something was removed; the rest of the sheet stays byte-identical.
// Independent of ApplyWarningPatch: callers must persist the result even when
// the rule itself is no longer patchable, because the extra block is exactly
// the size mismatch that triggers Steam's client reinstall.
inline bool RemoveLegacyBlock(std::string& css) {
    const std::size_t begin = css.find(kLegacyBegin);
    if (begin == std::string::npos)
        return false;
    std::size_t eraseEnd = css.find(kLegacyEnd, begin);
    if (eraseEnd == std::string::npos)
        return false; // truncated marker: leave the file alone
    eraseEnd += kLegacyEnd.size();
    if (eraseEnd < css.size() && css[eraseEnd] == '\n')
        ++eraseEnd;
    std::size_t eraseBegin = begin;
    if (eraseBegin > 0 && css[eraseBegin - 1] == '\n')
        --eraseBegin;
    css.erase(eraseBegin, eraseEnd - eraseBegin);
    return true;
}

} // namespace ac::steamui::css
