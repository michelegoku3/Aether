#include "cases.h"

#include <iterator>

#include "hooks/steamui/SteamUIStylePatch.h"

namespace {

constexpr const char* kClass = "TestHash_123";

std::string StockCss() {
    return ".a{color:red}." + std::string(kClass) +
           "{display:flex;flex-direction:row;align-items:center;gap:10px;color:orange}.b{color:blue}";
}

std::string SpJs() { return std::string("x=1,OldSharingWarning:\"") + kClass + "\",y=2"; }

std::string LegacyTail() {
    return "\n/* aether:hide-old-sharing:begin */\n." + std::string(kClass) +
           "{display:none!important}\n/* aether:hide-old-sharing:end */\n";
}

std::string ReadAll(const fs::path& path) {
    std::ifstream input(path, std::ios::binary);
    return std::string(std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>());
}

void WriteSteamTree(Fixture& f, const std::string& css, const std::string& spJs) {
    fs::create_directories(f.dir / "steamui" / "css");
    f.Write("steamui/sp.js", spJs);
    f.Write("steamui/css/sp.css", css);
}

} // namespace

void SteamUiPatchTest() {
    using ac::steamui::ApplyLegacySharingWarningPolicy;

    // Hide then restore: the file size never changes and the sheet round-trips.
    {
        Fixture f;
        const std::string stock = StockCss();
        WriteSteamTree(f, stock, SpJs());
        const fs::path cssPath = f.dir / "steamui" / "css" / "sp.css";

        ApplyLegacySharingWarningPolicy(f.dir.string(), true);
        const std::string hidden = ReadAll(cssPath);
        CHECK_EQ(hidden.size(), stock.size());
        CHECK(hidden.find("display:none") != std::string::npos);
        CHECK(hidden.find("/*aeth:hid*/") != std::string::npos);

        ApplyLegacySharingWarningPolicy(f.dir.string(), false);
        CHECK(ReadAll(cssPath) == stock);
    }

    // A legacy appended block is migrated away even when the rule cannot be
    // patched (sp.js maps a class that sp.css does not contain). Leaving it
    // would keep the size mismatch that triggers Steam's client reinstall.
    {
        Fixture f;
        const std::string stock = StockCss();
        fs::create_directories(f.dir / "steamui" / "css");
        f.Write("steamui/sp.js", "x=1,OldSharingWarning:\"NoSuchRule_9\",y=2");
        f.Write("steamui/css/sp.css", stock + LegacyTail());

        ApplyLegacySharingWarningPolicy(f.dir.string(), true);

        const std::string migrated = ReadAll(f.dir / "steamui" / "css" / "sp.css");
        CHECK(migrated == stock); // block removed, rule untouched
    }

    // Same when sp.js is unreadable or has no mapping at all.
    {
        Fixture f;
        const std::string stock = StockCss();
        fs::create_directories(f.dir / "steamui" / "css");
        f.Write("steamui/css/sp.css", stock + LegacyTail());

        ApplyLegacySharingWarningPolicy(f.dir.string(), true);

        CHECK(ReadAll(f.dir / "steamui" / "css" / "sp.css") == stock);
    }
    {
        Fixture f;
        const std::string stock = StockCss();
        fs::create_directories(f.dir / "steamui" / "css");
        f.Write("steamui/sp.js", "no mapping here");
        f.Write("steamui/css/sp.css", stock + LegacyTail());

        ApplyLegacySharingWarningPolicy(f.dir.string(), true);

        CHECK(ReadAll(f.dir / "steamui" / "css" / "sp.css") == stock);
    }

    // A path that was never resolved is a quiet no-op.
    {
        ApplyLegacySharingWarningPolicy(std::string(), true);
        ApplyLegacySharingWarningPolicy(std::string(), false);
    }
}
