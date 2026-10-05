#include "pch.h"
#include "utils/PatternEngine.h"
#include "utils/PatternCache.h"
#include "core/Logger.h"
#include <thread>

namespace ac::pattern {
    namespace {
        constexpr const char* kModule = "PatternEngine";
        using detail::PatternIndex;
        using detail::LoadModule;
        using detail::SweepTempFiles;
        PatternIndex* IndexFor(const std::string& module) {
            if (module == "steamclient") return &g_state.patterns.steamclient;
            if (module == "steamui") return &g_state.patterns.steamui;
            return nullptr;
        }

    }  // namespace

    bool Init() {
        AC_LOG_INFO(kModule, "Initialising.");
        CreateDirectoryA(g_state.patternDir.c_str(), nullptr);
        SweepTempFiles();

        g_state.steamuiPath = g_state.steamInstallPath + "\\steamui.dll";

        // Resolve both modules concurrently. On a fresh Steam build every cache
        // miss costs network round-trips; running them in parallel cuts the
        // critical path before hook installation in half (steamui's resolution
        // no longer waits behind steamclient's), so the hooks are in place
        // before Steam fires the one-shot LoadPackage(package 0) at startup.
        //
        // The two threads touch disjoint pattern indexes. They compute their
        // SHA into LOCAL strings on purpose: dllmain publishes
        // g_state.steamclientSha before spawning the concurrent IPC-spec
        // resolution, which reads it — writing the std::string here while that
        // thread reads it would be a data race. The local steamclient SHA is
        // identical to the published one; only steamuiSha is owned by Init.
        bool steamclientOk = false;
        bool steamuiOk = false;
        std::string clientSha;
        std::string uiSha;
        std::thread clientThread([&] {
            steamclientOk = LoadModule("steamclient", g_state.steamclientPath,
                clientSha, g_state.patterns.steamclient);
        });
        std::thread uiThread([&] {
            steamuiOk = LoadModule("steamui", g_state.steamuiPath,
                uiSha, g_state.patterns.steamui);
        });
        clientThread.join();
        uiThread.join();

        // StatusWriter is already active: protect the diagnostic publication.
        {
            std::lock_guard lock(g_state.statusMetadataMutex);
            g_state.steamuiSha = uiSha;
        }

        return steamclientOk || steamuiOk;
    }

    bool ReloadModuleIfMissing(const std::string& module) {
        PatternIndex* index = IndexFor(module);
        if (!index) return false;
        {
            std::shared_lock lock(g_state.patterns.mutex);
            if (!index->empty()) return false;  // no newly published table
        }

        const std::string* dllPath = (module == "steamui") ? &g_state.steamuiPath
                                                           : &g_state.steamclientPath;
        if (!dllPath || dllPath->empty()) return false;

        // LoadModule applies the provenance policy itself (see above): a cached
        // table is served without network when its sidecar says it already came
        // from the preferred source, and the upstream chain is consulted
        // otherwise. Re-running it here therefore picks up both a table that
        // arrived after init and, at most once per module, an upstream upgrade.
        std::string sha;
        PatternIndex candidate;
        if (!LoadModule(module, *dllPath, sha, candidate) || candidate.empty()) return false;
        {
            std::unique_lock lock(g_state.patterns.mutex);
            *index = std::move(candidate);
        }
        if (module == "steamui") {
            std::lock_guard lock(g_state.statusMetadataMutex);
            g_state.steamuiSha = sha;
        }
        return true;
    }

    bool HasModule(const std::string& module) {
        std::shared_lock lock(g_state.patterns.mutex);
        const auto* index = IndexFor(module);
        return index && !index->empty();
    }

}  // namespace ac::pattern
