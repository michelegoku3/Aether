#include "cases.h"

#include "core/Logger.h"
#include "core/Settings.h"

#include "utils/DeskPaths.h"

void Paths() {

    Fixture f;

    CHECK(ac::deskpaths::ReadDataDir(f.dir).empty());

    f.Write("desk_path.cfg", "\xEF\xBB\xBF  C:\\Desk with spaces\\AetherData \r\nignored");

    CHECK(ac::deskpaths::ReadDataDir(f.dir) == "C:\\Desk with spaces\\AetherData");

    f.Write("desk_path.cfg", "\r\n");

    CHECK(ac::deskpaths::ReadDataDir(f.dir).empty());

    f.Write("desk_path.cfg", std::string("bad\0path", 8));

    CHECK(ac::deskpaths::ReadDataDir(f.dir).empty());

    CHECK(ac::deskpaths::ConfigPath("data") == fs::path("data") / "config" / "aethercore.toml");
}

void Settings() {

    Fixture f;

    auto path = f.Write("settings.toml", "[presence]\ncustom_game_name='old'\nshowonline_apps=[1,2]\n");

    ac::Settings::Initialize(path.string());

    const auto old = ac::Settings::Snapshot();

    CHECK(old->presenceCustomGameName == "old");

    f.Write("settings.toml", "[presence]\ncustom_game_name='new'\nshowonline_apps=[3]\n");

    ac::Settings::ReloadIfModified(path.string());

    auto current = ac::Settings::Snapshot();

    CHECK(current->presenceCustomGameName == "new");

    CHECK(current->presenceShowOnlineApps == std::vector<std::uint32_t>{3});

    // Presence lists keep only positive integers (ParseAppList semantics).

    // NOTE: keep a custom_game_name here: the coherency spin below asserts a

    // non-empty name on every snapshot, so the pre-loop snapshot must have one.

    f.Write("settings.toml", "[presence]\ncustom_game_name='mixed'\nshowonline_apps=[0,-5,480,'abc']\n");

    ac::Settings::ReloadIfModified(path.string());

    CHECK(ac::Settings::Snapshot()->presenceShowOnlineApps == std::vector<std::uint32_t>{480});

    current = ac::Settings::Snapshot();

    CHECK(old->presenceCustomGameName == "old" && old->presenceShowOnlineApps.size() == 2);

    ac::Settings::ReloadIfModified(path.string());

    CHECK(ac::Settings::Snapshot() == current);

    f.Write("settings.toml", "[presence\ninvalid");

    ac::Settings::ReloadIfModified(path.string());

    CHECK(ac::Settings::Snapshot() == current);

    fs::remove(path);

    ac::Settings::ReloadIfModified(path.string());

    CHECK(ac::Settings::Snapshot() == current);

    std::atomic<bool> stop{false}, coherent{true};

    std::thread reader([&] {
        while (!stop) {

            const auto snap = ac::Settings::Snapshot();

            if (snap->presenceShowOnlineApps.empty())

                coherent = false;

            // Exercise references while the publisher replaces large allocations.

            const auto& name = snap->presenceCustomGameName;

            std::this_thread::yield();

            if (name.empty())

                coherent = false;
        }
    });

    for (int i = 0; i < 100; ++i) {

        f.Write("settings.toml",

                "[presence]\ncustom_game_name='revision" + std::to_string(i) + "'\nshowonline_apps=[4,5,6]\n");

        ac::Settings::ReloadIfModified(path.string());
    }

    stop = true;

    reader.join();

    CHECK(coherent);

    CHECK(ac::Settings::Snapshot()->presenceCustomGameName == "revision99");

    // Startup-only injection selection is strict, with auto as the default.

    bool valid = false;

    CHECK(ac::Settings::Load("nonexistent.toml", &valid).diversionMode == ac::DiversionMode::Auto);

    CHECK(!valid);

    auto modePath = f.Write("injection.toml", "[injection]\ndiversion_mode = 'copy'\n");

    CHECK(ac::Settings::Load(modePath.string(), &valid).diversionMode == ac::DiversionMode::Copy);

    CHECK(valid);

    f.Write("injection.toml", "[injection]\ndiversion_mode = 'live'\n");

    CHECK(ac::Settings::Load(modePath.string(), &valid).diversionMode == ac::DiversionMode::Live);

    CHECK(valid);

    f.Write("injection.toml", "[injection]\ndiversion_mode = 'invalid'\n");

    ac::Settings::Load(modePath.string(), &valid);

    CHECK(!valid);

    // F3: deprecated [manifest_fetch] is loud (ERROR + status record) but not
    // fatal: the table is ignored, everything else still parses.
    auto depPath = f.Write("deprecated.toml", "[manifest_fetch]\nurls = []\n[presence]\ncustom_game_name='kept'\n");
    const auto dep = ac::Settings::Load(depPath.string(), &valid);
    CHECK(valid);
    CHECK(dep.presenceCustomGameName == "kept");
    bool deprecatedSeen = false;
    for (const auto& e : ac::diag::Snapshot()) {
        if (e.category == "config_deprecated_manifest_fetch")
            deprecatedSeen = true;
    }
    CHECK(deprecatedSeen);
}
