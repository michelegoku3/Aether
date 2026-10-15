#include "MinHook.h"

#include "cases.h"

#include "core/HookManager.h"

void Registry() {

    ac::HookManager manager;

    manager.RecordMissed("late", ac::MissReason::PatternUnresolved);

    auto before = manager.Snapshot();

    CHECK(before.missed.size() == 1);

    manager.RegisterHook("late", nullptr, nullptr, nullptr);

    CHECK(manager.InstallAll());

    CHECK(manager.Snapshot().installed.size() == 1 && manager.Snapshot().missed.empty());

    CHECK(before.missed.size() == 1); // owned immutable copy

    std::atomic<bool> stop{false}, coherent{true};

    std::thread reader([&] {
        while (!stop)

            if (manager.Snapshot().installed.empty())

                coherent = false;
    });

    for (int i = 0; i < 1000; ++i) {

        const auto name = "hook" + std::to_string(i);

        manager.RecordMissed(name, ac::MissReason::PatternUnresolved);

        manager.RegisterHook(name, nullptr, nullptr, nullptr);
    }

    CHECK(manager.InstallAll());

    stop = true;

    reader.join();

    CHECK(coherent);

    CHECK(manager.Snapshot().installed.size() == 1001);

    CHECK(manager.InstallAll());

    CHECK(manager.Snapshot().installed.size() == 1001);

    manager.UninstallAll();

    manager.UninstallAll();

    CHECK(manager.Snapshot().installed.empty());

    ac::HookManager retried;

    retried.RegisterHook("LoadModuleWithPath", nullptr, nullptr, nullptr);

    testCreateFails = true;

    CHECK(retried.InstallAll());

    CHECK(retried.InstallAll());

    CHECK(retried.Snapshot().missed.size() == 1); // no duplicate retry misses

    testCreateFails = false;

    CHECK(retried.InstallAll());

    CHECK(retried.Snapshot().installed.size() == 1);

    CHECK(retried.Snapshot().missed.empty());
}
