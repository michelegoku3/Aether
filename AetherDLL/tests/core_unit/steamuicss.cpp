#include "cases.h"

#include "core/SteamUiCssText.h"

namespace {

constexpr const char* kClass = "IPyS4NVb7QjBlTALA_8U8";

// Mirrors the real sp.css neighbourhood: two layout variants of the class, then
// the orange row rule (the target), a child rule, and an unrelated orange rule
// that must never be touched.
const std::string kFragment = ".unrelated{margin-top:10px;min-height:36px;font-size:14px}"
                              ".BasicUI .IPyS4NVb7QjBlTALA_8U8{margin-top:8px;height:48px;gap:6px}"
                              ".IPyS4NVb7QjBlTALA_8U8{margin-top:10px;min-height:36px;font-size:14px}"
                              ".IPyS4NVb7QjBlTALA_8U8{display:flex;flex-direction:row;align-items:center;gap:10px;"
                              "color:orange}"
                              ".IPyS4NVb7QjBlTALA_8U8 .child{color:#6dcff6;cursor:pointer}"
                              ".other{color:orange}";

const std::string kLegacyTail =
    "\n/* aether:hide-old-sharing:begin */\n.IPyS4NVb7QjBlTALA_8U8{display:none!important}\n"
    "/* aether:hide-old-sharing:end */\n";

} // namespace

void SteamUiCssTest() {
    using namespace ac::steamui::css;

    // ---- hide -------------------------------------------------------------

    {
        std::string css = kFragment;
        const std::size_t before = css.size();
        CHECK(ApplyWarningPatch(css, kClass, true) == WarningPatch::Changed);
        CHECK_EQ(css.size(), before); // size-preserving: Steam verifies sizes
        CHECK(css.find("display:none") != std::string::npos);
        CHECK(css.find(kHideMarker) != std::string::npos);
        CHECK(css.find("display:flex") == std::string::npos);
        // The child rule and the unrelated orange rule keep their own colour.
        CHECK(css.find(kOriginalColor) != std::string::npos);
        CHECK(css.find(".other{color:orange}") != std::string::npos);
        // Only the body changed: the rule around it is intact.
        CHECK(css.find(".IPyS4NVb7QjBlTALA_8U8{display:none;") != std::string::npos);
    }

    // Idempotent: a second hide is a no-op (no write, no size change).
    {
        std::string css = kFragment;
        CHECK(ApplyWarningPatch(css, kClass, true) == WarningPatch::Changed);
        const std::string patched = css;
        CHECK(ApplyWarningPatch(css, kClass, true) == WarningPatch::Unchanged);
        CHECK(css == patched);
    }

    // ---- restore ----------------------------------------------------------

    {
        std::string css = kFragment;
        CHECK(ApplyWarningPatch(css, kClass, true) == WarningPatch::Changed);
        const std::size_t patched = css.size();
        CHECK(ApplyWarningPatch(css, kClass, false) == WarningPatch::Changed);
        CHECK_EQ(css.size(), patched);
        CHECK(css == kFragment); // byte-identical round trip
    }

    // Restoring an untouched sheet is a no-op.
    {
        std::string css = kFragment;
        CHECK(ApplyWarningPatch(css, kClass, false) == WarningPatch::Unchanged);
        CHECK(css == kFragment);
    }

    // ---- declaration boundaries -------------------------------------------

    // `color:orange` is a substring of `background-color:orange`, which is a
    // different declaration: selecting it would corrupt the property name.
    {
        std::string css = ".X{display:flex;background-color:orange}";
        const std::string before = css;
        CHECK(ApplyWarningPatch(css, "X", true) == WarningPatch::NotApplicable);
        CHECK(css == before);
    }
    {
        std::string css = ".X{background-color:orange;display:flex}";
        const std::string before = css;
        CHECK(ApplyWarningPatch(css, "X", true) == WarningPatch::NotApplicable);
        CHECK(css == before);
    }
    {
        std::string css = ".X{outline-color:orange;display:flex}";
        const std::string before = css;
        CHECK(ApplyWarningPatch(css, "X", true) == WarningPatch::NotApplicable);
        CHECK(css == before);
    }

    // A declaration may follow whitespace and may carry !important.
    {
        std::string css = ".X{display:flex; color:orange !important}";
        const std::string before = css;
        CHECK(ApplyWarningPatch(css, "X", true) == WarningPatch::Changed);
        CHECK_EQ(css.size(), before.size());
        CHECK(css.find("display:none") != std::string::npos);
        CHECK(css.find(kHideMarker) != std::string::npos);
        CHECK(css.find("!important") != std::string::npos);
    }

    // ---- defensive paths --------------------------------------------------

    {
        std::string css = kFragment;
        CHECK(ApplyWarningPatch(css, "NoSuchClass", true) == WarningPatch::NotApplicable);
        CHECK(css == kFragment);
    }
    {
        // Duplicated tokens: not the rule we understand, leave it alone.
        std::string css = ".X{display:flex;display:flex;color:orange}";
        const std::string before = css;
        CHECK(ApplyWarningPatch(css, "X", true) == WarningPatch::NotApplicable);
        CHECK(css == before);
    }
    {
        // The class exists but carries no orange row (layout variant only).
        std::string css = ".X{margin-top:8px;height:48px;gap:6px}";
        const std::string before = css;
        CHECK(ApplyWarningPatch(css, "X", true) == WarningPatch::NotApplicable);
        CHECK(css == before);
    }
    {
        // A truncated rule (no closing brace) is never edited.
        std::string css = ".X{display:flex;color:orange";
        const std::string before = css;
        CHECK(ApplyWarningPatch(css, "X", true) == WarningPatch::NotApplicable);
        CHECK(css == before);
    }

    // ---- legacy marker block migration ------------------------------------

    {
        std::string css = kFragment + kLegacyTail;
        CHECK(RemoveLegacyBlock(css));
        CHECK_EQ(css.size(), kFragment.size());
        CHECK(css == kFragment);
    }
    {
        std::string css = kFragment;
        CHECK(!RemoveLegacyBlock(css));
        CHECK(css == kFragment);
    }
    {
        // Truncated block: leave the sheet alone rather than guess the span.
        std::string css = kFragment + "\n/* aether:hide-old-sharing:begin */\n.X{display:none!important}\n";
        const std::string before = css;
        CHECK(!RemoveLegacyBlock(css));
        CHECK(css == before);
    }
    {
        // Migration restores the stock size before the in-place patch runs.
        std::string css = kFragment + kLegacyTail;
        CHECK(RemoveLegacyBlock(css));
        const std::size_t stock = css.size();
        CHECK(ApplyWarningPatch(css, kClass, true) == WarningPatch::Changed);
        CHECK_EQ(css.size(), stock);
    }
}
