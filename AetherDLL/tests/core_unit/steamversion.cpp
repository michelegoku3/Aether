#include "cases.h"

#include "core/SteamVersion.h"
#include "utils/Strings.h"

// The Steam client channel comes from <steam>\package\beta: the file holds the
// selected branch ("publicbeta") and is absent or empty on stable. Only a clean,
// non-empty branch name may raise the startup notice, so anything else has to
// read as stable.
void SteamBetaChannelTest() {
    using ac::ParseBetaBranch;

    CHECK(ParseBetaBranch("").empty());
    CHECK(ParseBetaBranch("   \r\n\t").empty());
    CHECK(ParseBetaBranch("\xEF\xBB\xBF").empty());

    CHECK_EQ(ParseBetaBranch("publicbeta"), std::string("publicbeta"));
    CHECK_EQ(ParseBetaBranch("publicbeta\r\n"), std::string("publicbeta"));
    CHECK_EQ(ParseBetaBranch("\xEF\xBB\xBF publicbeta \n"), std::string("publicbeta"));
    CHECK_EQ(ParseBetaBranch("beta"), std::string("beta"));

    // Not a branch name: a stale or foreign marker must never raise the notice,
    // and the name reaches the log, so control characters are rejected too.
    CHECK(ParseBetaBranch("public beta!").empty());
    CHECK(ParseBetaBranch("beta\nversion 2").empty());
    CHECK(ParseBetaBranch(std::string("publicbeta\0extra", 16)).empty());

    // The marker file is the only input. Absent, empty, oversized and non-text
    // markers read as stable.
    Fixture fx;
    // Runtime paths are UTF-8, so the fixture path is handed over the same way
    // production hands over g_state.steamInstallPath.
    const std::string root = ac::strings::Narrow(fx.dir.wstring());
    const auto writeMarker = [](const fs::path& root, const std::string& data) {
        fs::create_directories(root / "package");
        std::ofstream file(root / "package" / "beta", std::ios::binary);
        file << data;
    };

    CHECK(ac::DetectSteamBetaBranch(root).empty());

    writeMarker(fx.dir, "");
    CHECK(ac::DetectSteamBetaBranch(root).empty());

    writeMarker(fx.dir, "publicbeta\r\n");
    CHECK_EQ(ac::DetectSteamBetaBranch(root), std::string("publicbeta"));

    writeMarker(fx.dir, "public beta!");
    CHECK(ac::DetectSteamBetaBranch(root).empty());

    writeMarker(fx.dir, std::string(65, 'b'));
    CHECK(ac::DetectSteamBetaBranch(root).empty());

    // Runtime paths are UTF-8: a non-ASCII Steam root must still resolve.
    const fs::path unicodeRoot = fx.dir / L"\u0141ukasz";
    writeMarker(unicodeRoot, "publicbeta\n");
    CHECK_EQ(ac::DetectSteamBetaBranch(ac::strings::Narrow(unicodeRoot.wstring())), std::string("publicbeta"));
}
