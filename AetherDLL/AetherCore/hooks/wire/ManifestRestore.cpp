#include "pch.h"
#include "core/Workers.h"
#include "hooks/wire/ManifestRestore.h"

#include <atomic>
#include <cctype>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <regex>
#include <set>
#include <string>
#include <thread>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/license/LicenseManager.h"
#include "hooks/wire/BackupIo.h"
#include "scripting/ScriptEngine.h"
#include "utils/Paths.h"

namespace ac::hooks::ManifestRestore {
namespace {

namespace fs = std::filesystem;

constexpr const char* kModule = "ManifestRestore";

// A single full scan per Steam process. ACF events use
// RestoreMissingManifestsForApp() instead, which may run multiple times.
std::atomic<bool> s_started{false};

bool HasManifestExtension(const fs::path& path) { return paths::HasExtension(path, ".manifest"); }

bool IsLuaPath(const fs::path& path) { return paths::HasExtension(path, ".lua"); }

// A destination counts as "missing": absent or empty (0 bytes). An empty
// manifest is never valid, so it is restored as if missing. Everything else
// is left untouched: never overwrite an existing file.
bool IsMissingOrEmpty(const fs::path& dest) {
    std::error_code ec;
    if (!fs::exists(dest, ec) || ec)
        return true;
    const auto size = fs::file_size(dest, ec);
    return ec ? true : (size == 0);
}

// Outcome of one shared-contract backup copy.
enum class BackupCopyOutcome {
    Present, // destination already non-empty (or completed by the other writer)
    Copied,  // copied and verified by this call
    Failed,  // destination is still missing/empty
};

// The AetherDLL twin of AetherDesk's `backup_referenced_manifests`
// (core/backup.rs). Both writers follow the SAME contract, so whichever
// process sees a file first produces the same backup tree:
// 1. only files referenced by the current Lua are copied (callers filter);
// 2. a non-empty destination is never overwritten (the file name is the
//    manifest's complete depot/GID identity);
// 3. a zero-byte destination placeholder is replaced;
// 4. the copy is atomic (temp + replace) and size-verified;
// 5. a failed copy re-checks the destination: if AetherDesk completed the
//    same copy in the race window, the file counts as present.
BackupCopyOutcome CopyAtomicallyIfMissing(const fs::path& source, const fs::path& destination) {
    if (!IsMissingOrEmpty(destination))
        return BackupCopyOutcome::Present; // rule 2

    const fs::path temporary = backup::io::UniqueStagingPath(destination.string());
    std::error_code copyEc;
    fs::copy_file(source, temporary, fs::copy_options::overwrite_existing, copyEc);
    // AtomicReplace uses MOVEFILE_REPLACE_EXISTING, which also covers the
    // zero-byte placeholder of rule 3 in one step.
    if (copyEc || !backup::io::AtomicReplace(temporary.string(), destination.string())) {
        std::error_code cleanupEc;
        fs::remove(temporary, cleanupEc);
        return IsMissingOrEmpty(destination) ? BackupCopyOutcome::Failed // rule 5
                                             : BackupCopyOutcome::Present;
    }
    // Rule 4: the committed copy must match the source size.
    // Distinct source/destination errors: a source stat failure must not mask
    // the destination result, and the log names the failing side.
    const auto checked = backup::io::VerifySameSize(source.string(), destination.string());
    if (!checked.ok) {
        AC_LOG_WARN(kModule, "Backup copy failed size verification: %s (src=%s dst=%s).", destination.string().c_str(),
                    checked.srcError.empty() ? "ok" : checked.srcError.c_str(),
                    checked.dstError.empty() ? "ok" : checked.dstError.c_str());
        return BackupCopyOutcome::Failed;
    }
    return BackupCopyOutcome::Copied;
}

// operator+= and RestoreLuaDirectory live outside the anonymous namespace:
// the latter is a test seam (declared in ManifestRestore.h), so it needs
// external linkage. Helpers above stay anonymous (internal).
} // namespace

RestoreStats& operator+=(RestoreStats& self, const RestoreStats& other) {
    self.scanned += other.scanned;
    self.restored += other.restored;
    self.present += other.present;
    self.errors += other.errors;
    return self;
}

// Copies only manifests that are referenced by the selected app's backup.
// The filename() boundary is intentional: backup entries are flat and can
// never escape Steam\depotcache through a crafted path.
RestoreStats RestoreLuaDirectory(const fs::path& luaDir, const fs::path& depotcache) {
    RestoreStats stats;
    std::error_code ec;
    if (!fs::is_directory(luaDir, ec) || ec)
        return stats;

    fs::directory_iterator it(luaDir, ec);
    if (ec)
        return stats;
    for (const auto& entry : it) {
        std::error_code fileEc;
        if (!entry.is_regular_file(fileEc) || fileEc)
            continue;
        const fs::path& src = entry.path();
        if (!HasManifestExtension(src))
            continue;
        ++stats.scanned;

        const fs::path dest = depotcache / src.filename();
        if (!IsMissingOrEmpty(dest)) {
            ++stats.present;
            continue;
        }

        // `none` intentionally never overwrites a valid manifest. Remove only
        // the zero-byte placeholder that IsMissingOrEmpty classified as
        // missing; this also makes the empty-file recovery path effective on
        // Windows where rename/copy does not replace by default.
        if (fs::exists(dest, fileEc)) {
            fs::remove(dest, fileEc);
        }
        if (!fileEc) {
            fs::copy_file(src, dest, fs::copy_options::none, fileEc);
        }
        if (fileEc) {
            // Benign RACE: the file appeared in the meantime (Steam or another
            // worker). Recheck before reporting the error.
            if (!IsMissingOrEmpty(dest)) {
                ++stats.present;
            } else {
                ++stats.errors;
                AC_LOG_DEBUG(kModule, "Cannot restore %s (%s).", dest.string().c_str(), fileEc.message().c_str());
            }
        } else {
            // A torn write (AV quarantine, disk stall) leaves a truncated but
            // non-empty manifest that IsMissingOrEmpty would accept as
            // present. Verify against the source like the backup path does.
            const auto checked = backup::io::VerifySameSize(src.string(), dest.string());
            if (!checked.ok) {
                ++stats.errors;
                AC_LOG_DEBUG(kModule, "Restored size mismatch %s (src=%s dst=%s).", dest.string().c_str(),
                             checked.srcError.empty() ? "ok" : checked.srcError.c_str(),
                             checked.dstError.empty() ? "ok" : checked.dstError.c_str());
            } else {
                ++stats.restored;
            }
        }
    }
    return stats;
}

namespace {

bool ParseNumericAppId(const std::string& stem, std::uint32_t& out) {
    if (stem.empty())
        return false;
    unsigned long long value = 0;
    for (char c : stem) {
        if (c < '0' || c > '9')
            return false;
        value = value * 10u + static_cast<unsigned long long>(c - '0');
        if (value > UINT32_MAX)
            return false;
    }
    if (value == 0)
        return false;
    out = static_cast<std::uint32_t>(value);
    return true;
}

// Steam startup may be the first process that sees a new manifest in
// depotcache (for example when AetherDesk was not opened). Mirror every local
// Lua and the manifest files referenced by it before the restore pass runs.
// This is deliberately local-only: it never contacts a provider and never fabricates
// a manifest that is not already present in Steam's depotcache.
void BackupReferencedManifestsAtStartup() {
    const auto settings = Settings::Snapshot();
    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty()) {
        AC_LOG_DEBUG(kModule, "Manifest backup skipped: AetherData unavailable.");
        return;
    }

    const fs::path depotcache = fs::path(g_state.steamInstallPath) / "depotcache";
    const fs::path backupRoot = fs::path(deskData) / "backup";
    // Compiled once per process: std::regex construction is expensive and can
    // throw regex_error on pathological patterns; the scan below guards it.
    // NOLINTNEXTLINE(bugprone-throwing-static-initialization)
    static const std::regex pinRegex(R"(setManifestid\s*\(\s*([0-9]+)\s*,\s*["']([0-9]+)["'])",
                                     std::regex_constants::icase | std::regex_constants::optimize);

    std::vector<fs::path> luaRoots{fs::path(g_state.luaDir)};
    for (const std::string& extra : settings->luaExtraPaths) {
        luaRoots.emplace_back(extra);
    }

    std::error_code ec;
    if (!fs::is_directory(depotcache, ec) || ec) {
        AC_LOG_DEBUG(kModule, "Manifest backup skipped: depotcache directory is unavailable.");
        return;
    }

    std::size_t luaScanned = 0;
    std::size_t luaCopied = 0;
    std::size_t referenced = 0;
    std::size_t copied = 0;
    std::size_t alreadyPresent = 0;
    std::size_t missing = 0;

    for (const fs::path& luaRoot : luaRoots) {
        if (!fs::is_directory(luaRoot, ec) || ec)
            continue;
        fs::directory_iterator luaIt(luaRoot, ec);
        if (ec)
            continue;

        for (const auto& luaEntry : luaIt) {
            std::error_code fileEc;
            if (!luaEntry.is_regular_file(fileEc) || fileEc || !IsLuaPath(luaEntry.path()))
                continue;
            std::uint32_t appId = 0;
            if (!ParseNumericAppId(luaEntry.path().stem().string(), appId))
                continue;
            ++luaScanned;

            std::ifstream input(luaEntry.path(), std::ios::binary);
            if (!input.is_open())
                continue;
            const std::string content((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
            const fs::path appLuaBackup = backupRoot / std::to_string(appId) / "lua";
            fs::create_directories(appLuaBackup, ec);
            if (ec)
                continue;

            // Steam can be started without AetherDesk. Preserve a newly seen
            // Lua through the shared-contract copier; AetherDesk's richer
            // startup sync owns history/version decisions.
            const fs::path luaDestination = appLuaBackup / (std::to_string(appId) + ".lua");
            switch (CopyAtomicallyIfMissing(luaEntry.path(), luaDestination)) {
            case BackupCopyOutcome::Copied:
                ++luaCopied;
                break;
            case BackupCopyOutcome::Present:
                break;
            case BackupCopyOutcome::Failed:
                AC_LOG_DEBUG(kModule, "Cannot backup Lua %s.", luaDestination.string().c_str());
                break;
            }

            std::set<std::string> filenames;
            try {
                for (std::sregex_iterator match(content.begin(), content.end(), pinRegex), end; match != end; ++match) {
                    const std::string depot = (*match)[1].str();
                    const std::string gid = (*match)[2].str();
                    std::string key;
                    key.reserve(depot.size() + gid.size() + 10);
                    key += depot;
                    key += '_';
                    key += gid;
                    key += ".manifest";
                    filenames.insert(std::move(key));
                }
            } catch (const std::regex_error& e) {
                AC_LOG_WARN(kModule, "Pin scan failed (%s); skipping manifest backup for this file.", e.what());
                continue;
            }
            referenced += filenames.size();

            for (const std::string& filename : filenames) {
                const fs::path source = depotcache / filename;
                if (!fs::is_regular_file(source, ec) || ec) {
                    ++missing;
                    continue;
                }
                switch (CopyAtomicallyIfMissing(source, appLuaBackup / filename)) {
                case BackupCopyOutcome::Copied:
                    ++copied;
                    break;
                case BackupCopyOutcome::Present:
                    ++alreadyPresent;
                    break;
                case BackupCopyOutcome::Failed:
                    AC_LOG_DEBUG(kModule, "Cannot backup %s.", source.string().c_str());
                    break;
                }
            }
        }
    }

    AC_LOG_INFO(kModule,
                "Lua/manifest backup complete: lua=%zu copied_lua=%zu referenced=%zu "
                "copied_manifests=%zu already_present=%zu missing_from_depotcache=%zu.",
                luaScanned, luaCopied, referenced, copied, alreadyPresent, missing);
}

void RestoreAllOnce() {
    const auto settings = Settings::Snapshot();
    if (!settings->manifestRestoreOnStartup) {
        AC_LOG_DEBUG(kModule, "Restore on startup disabled by config; skipping.");
        return;
    }

    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty()) {
        AC_LOG_DEBUG(kModule, "Restore skipped: AetherData unavailable (no AetherDesk bridge).");
        return;
    }

    const fs::path backupRoot = fs::path(deskData) / "backup";
    const fs::path depotcache = fs::path(g_state.steamInstallPath) / "depotcache";
    std::error_code ec;
    fs::create_directories(depotcache, ec);
    if (ec) {
        AC_LOG_WARN(kModule, "Restore skipped: cannot create %s (%s).", depotcache.string().c_str(),
                    ec.message().c_str());
        return;
    }
    if (!fs::is_directory(backupRoot, ec) || ec) {
        AC_LOG_INFO(kModule, "No backup root at %s; nothing to restore.", backupRoot.string().c_str());
        return;
    }

    RestoreStats total;
    fs::directory_iterator appIt(backupRoot, ec);
    if (ec) {
        AC_LOG_WARN(kModule, "Cannot list %s (%s).", backupRoot.string().c_str(), ec.message().c_str());
        return;
    }
    for (const auto& appEntry : appIt) {
        std::error_code entryEc;
        if (!appEntry.is_directory(entryEc) || entryEc)
            continue;
        total += RestoreLuaDirectory(appEntry.path() / "lua", depotcache);
    }

    AC_LOG_INFO(kModule,
                "Restore complete: scanned=%zu restored=%zu already_present=%zu errors=%zu "
                "(%s -> %s).",
                total.scanned, total.restored, total.present, total.errors, backupRoot.string().c_str(),
                depotcache.string().c_str());
}

void RestoreAppOnce(std::uint32_t appId) {
    const auto settings = Settings::Snapshot();
    if (!settings->manifestRestoreOnStartup) {
        AC_LOG_DEBUG(kModule, "Restore after ACF removal disabled by config; app=%u.", appId);
        return;
    }

    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty())
        return;
    const fs::path backupRoot = fs::path(deskData) / "backup";
    const fs::path depotcache = fs::path(g_state.steamInstallPath) / "depotcache";
    std::error_code ec;
    fs::create_directories(depotcache, ec);
    if (ec)
        return;

    const RestoreStats stats = RestoreLuaDirectory(backupRoot / std::to_string(appId) / "lua", depotcache);
    AC_LOG_INFO(kModule,
                "Restore after appmanifest_%u.acf removal: scanned=%zu restored=%zu "
                "already_present=%zu errors=%zu.",
                appId, stats.scanned, stats.restored, stats.present, stats.errors);

    if (stats.restored == 0)
        return;

    // Reinstalling the same Lua and manifest package works because Steam sees a
    // Lua file change: the script is parsed again, package-0 licenses are
    // refreshed, and the newly restored depotcache files become visible to the
    // depot manager. ACF removal previously restored only bytes on disk, leaving
    // the in-memory Lua/license state stale. Re-run that same refresh sequence
    // after the targeted restore, without downloading anything.
    const fs::path luaPath = fs::path(g_state.luaDir) / (std::to_string(appId) + ".lua");
    std::error_code luaEc;
    if (!fs::is_regular_file(luaPath, luaEc) || luaEc) {
        AC_LOG_WARN(kModule,
                    "Restored manifests for app %u but live Lua is unavailable at %s; "
                    "Steam state was not hot-reloaded.",
                    appId, luaPath.string().c_str());
        return;
    }

    ac::script::ParseFile(luaPath.string());
    ac::hooks::LicenseManager::NotifyLicenseChanged();
    AC_LOG_INFO(kModule, "Refreshed Lua/license state after restoring manifests for AppID %u.", appId);
}

} // namespace

void RestoreMissingManifestsAtStartup() {
    if (s_started.exchange(true))
        return; // once per process
    try {
        // Backup runs before restore: when AetherDesk was never started, Steam
        // can still capture the manifests still present in depotcache and keep
        // them for a future uninstall.
        // Task queue instead of detach(): the work stays observable and is
        // drained/joined on explicit shutdown.
        if (!workers::Submit([] {
                try {
                    BackupReferencedManifestsAtStartup();
                    RestoreAllOnce();
                } catch (const std::exception& e) {
                    AC_LOG_ERROR(kModule, "Startup manifest worker failed: %s.", e.what());
                } catch (...) {
                    AC_LOG_ERROR(kModule, "Startup manifest worker failed with unknown exception.");
                }
            })) {
            AC_LOG_WARN(kModule, "Startup manifest worker rejected (workers shut down).");
        }
    } catch (const std::exception& e) {
        AC_LOG_ERROR(kModule, "Cannot start restore worker: %s.", e.what());
    } catch (...) {
        AC_LOG_ERROR(kModule, "Cannot start restore worker (unknown exception).");
    }
}

void RestoreMissingManifestsForApp(std::uint32_t appId) {
    if (appId == 0)
        return;
    try {
        // Called from the DirWatch thread after the ACF event debounce; keeping
        // it synchronous stops Steam from exiting mid-transfer.
        RestoreAppOnce(appId);
    } catch (const std::exception& e) {
        AC_LOG_ERROR(kModule, "Restore for AppID %u failed: %s.", appId, e.what());
    } catch (...) {
        AC_LOG_ERROR(kModule, "Restore for AppID %u failed with unknown exception.", appId);
    }
}

void BackupManifestAfterGeneration(std::uint32_t depotId, std::uint64_t gid) {
    const auto settings = Settings::Snapshot();
    if (depotId == 0 || gid == 0)
        return;
    try {
        const std::string deskData = backup::io::CachedDeskDataDir();
        if (deskData.empty())
            return;

        std::error_code ec;
        const fs::path source = fs::path(g_state.steamInstallPath) / "depotcache" /
                                (std::to_string(depotId) + "_" + std::to_string(gid) + ".manifest");
        if (!fs::is_regular_file(source, ec) || ec)
            return;

        const fs::path backupRoot = fs::path(deskData) / "backup";
        const std::string depotText = std::to_string(depotId);
        const std::string gidText = std::to_string(gid);
        // Same regex as the startup backup: also catches commented pins
        // (disabled setManifestid = game in update mode). Compiled once per
        // process, like the startup path.
        // NOLINTNEXTLINE(bugprone-throwing-static-initialization)
        static const std::regex pinRegex(R"(setManifestid\s*\(\s*([0-9]+)\s*,\s*["']([0-9]+)["'])",
                                         std::regex_constants::icase | std::regex_constants::optimize);

        std::vector<fs::path> luaRoots{fs::path(g_state.luaDir)};
        for (const std::string& extra : settings->luaExtraPaths) {
            luaRoots.emplace_back(extra);
        }

        std::size_t copied = 0;
        for (const fs::path& luaRoot : luaRoots) {
            if (!fs::is_directory(luaRoot, ec) || ec)
                continue;
            fs::directory_iterator luaIt(luaRoot, ec);
            if (ec)
                continue;
            for (const auto& luaEntry : luaIt) {
                std::error_code fileEc;
                if (!luaEntry.is_regular_file(fileEc) || fileEc || !IsLuaPath(luaEntry.path())) {
                    continue;
                }
                std::uint32_t appId = 0;
                if (!ParseNumericAppId(luaEntry.path().stem().string(), appId))
                    continue;

                std::ifstream input(luaEntry.path(), std::ios::binary);
                if (!input.is_open())
                    continue;
                const std::string content((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
                bool referenced = false;
                for (std::sregex_iterator match(content.begin(), content.end(), pinRegex), end; match != end; ++match) {
                    if ((*match)[1].str() == depotText && (*match)[2].str() == gidText) {
                        referenced = true;
                        break;
                    }
                }
                if (!referenced)
                    continue;

                const fs::path appLuaBackup = backupRoot / std::to_string(appId) / "lua";
                fs::create_directories(appLuaBackup, ec);
                if (ec)
                    continue;
                if (CopyAtomicallyIfMissing(source, appLuaBackup / source.filename()) == BackupCopyOutcome::Copied) {
                    ++copied;
                }
            }
        }
        if (copied > 0) {
            AC_LOG_INFO(kModule, "Generated manifest depot=%u gid=%llu archived into %zu app backup(s).", depotId,
                        static_cast<unsigned long long>(gid), copied);
        }
    } catch (const std::exception& e) {
        AC_LOG_DEBUG(kModule, "Post-generation manifest backup failed depot=%u: %s.", depotId, e.what());
    } catch (...) {
        AC_LOG_DEBUG(kModule, "Post-generation manifest backup failed depot=%u.", depotId);
    }
}

} // namespace ac::hooks::ManifestRestore
