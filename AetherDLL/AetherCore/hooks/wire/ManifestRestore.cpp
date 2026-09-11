#include "pch.h"
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
#include "hooks/wire/BackupIo.h"

namespace ac::hooks::ManifestRestore {
namespace {

namespace fs = std::filesystem;

constexpr const char* kModule = "ManifestRestore";

// Una sola scansione completa per processo di Steam. Gli eventi ACF usano
// invece RestoreMissingManifestsForApp(), che può essere richiamato più volte.
std::atomic<bool> s_started{false};

bool HasExtension(const fs::path& path, const char* wanted) {
    std::string ext = path.extension().string();
    std::string expected = wanted;
    if (ext.size() != expected.size()) return false;
    for (char& c : ext) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    for (char& c : expected) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return ext == expected;
}

bool HasManifestExtension(const fs::path& path) {
    return HasExtension(path, ".manifest");
}

bool IsLuaPath(const fs::path& path) {
    return HasExtension(path, ".lua");
}

// Destino considerato "mancante": assente oppure vuoto (0 byte). Un manifest
// vuoto non è mai valido, quindi va ripristinato come se mancasse. Tutto il
// resto viene lasciato intatto: mai sovrascrivere un file esistente.
bool IsMissingOrEmpty(const fs::path& dest) {
    std::error_code ec;
    if (!fs::exists(dest, ec) || ec) return true;
    const auto size = fs::file_size(dest, ec);
    return ec ? true : (size == 0);
}

struct RestoreStats {
    std::size_t scanned = 0;
    std::size_t restored = 0;
    std::size_t present = 0;
    std::size_t errors = 0;

    RestoreStats& operator+=(const RestoreStats& other) {
        scanned += other.scanned;
        restored += other.restored;
        present += other.present;
        errors += other.errors;
        return *this;
    }
};

// Copies only manifests that are referenced by the selected app's backup.
// The filename() boundary is intentional: backup entries are flat and can
// never escape Steam\depotcache through a crafted path.
RestoreStats RestoreLuaDirectory(const fs::path& luaDir, const fs::path& depotcache) {
    RestoreStats stats;
    std::error_code ec;
    if (!fs::is_directory(luaDir, ec) || ec) return stats;

    fs::directory_iterator it(luaDir, ec);
    if (ec) return stats;
    for (const auto& entry : it) {
        std::error_code fileEc;
        if (!entry.is_regular_file(fileEc) || fileEc) continue;
        const fs::path src = entry.path();
        if (!HasManifestExtension(src)) continue;
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
            // RACE benigna: il file è comparso nel frattempo (Steam o un altro
            // worker). Ricontrolla prima di segnalare l'errore.
            if (!IsMissingOrEmpty(dest)) {
                ++stats.present;
            } else {
                ++stats.errors;
                AC_LOG_DEBUG(kModule, "Cannot restore %s (%s).",
                             dest.string().c_str(), fileEc.message().c_str());
            }
        } else {
            ++stats.restored;
        }
    }
    return stats;
}

bool ParseNumericAppId(const std::string& stem, std::uint32_t& out) {
    if (stem.empty()) return false;
    unsigned long long value = 0;
    for (char c : stem) {
        if (c < '0' || c > '9') return false;
        value = value * 10u + static_cast<unsigned long long>(c - '0');
        if (value > UINT32_MAX) return false;
    }
    if (value == 0) return false;
    out = static_cast<std::uint32_t>(value);
    return true;
}

// Steam startup may be the first process that sees a new manifest in
// depotcache (for example when AetherDesk was not opened). Mirror every local
// Lua and the manifest files referenced by it before the restore pass runs.
// This is deliberately local-only: it never contacts a provider and never fabricates
// a manifest that is not already present in Steam's depotcache.
void BackupReferencedManifestsAtStartup() {
    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty()) {
        AC_LOG_DEBUG(kModule, "Manifest backup skipped: AetherData unavailable.");
        return;
    }

    const fs::path depotcache = fs::path(g_state.steamInstallPath) / "depotcache";
    const fs::path backupRoot = fs::path(deskData) / "backup";
    const std::regex pinRegex(
        R"(setManifestid\s*\(\s*([0-9]+)\s*,\s*["']([^"']+)["'])",
        std::regex_constants::icase);

    std::vector<fs::path> luaRoots{fs::path(g_state.luaDir)};
    for (const std::string& extra : g_state.settings.luaExtraPaths) {
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
        if (!fs::is_directory(luaRoot, ec) || ec) continue;
        fs::directory_iterator luaIt(luaRoot, ec);
        if (ec) continue;

        for (const auto& luaEntry : luaIt) {
            std::error_code fileEc;
            if (!luaEntry.is_regular_file(fileEc) || fileEc || !IsLuaPath(luaEntry.path())) continue;
            std::uint32_t appId = 0;
            if (!ParseNumericAppId(luaEntry.path().stem().string(), appId)) continue;
            ++luaScanned;

            std::ifstream input(luaEntry.path(), std::ios::binary);
            if (!input.is_open()) continue;
            const std::string content((std::istreambuf_iterator<char>(input)),
                                      std::istreambuf_iterator<char>());
            const fs::path appLuaBackup = backupRoot / std::to_string(appId) / "lua";
            fs::create_directories(appLuaBackup, ec);
            if (ec) continue;

            // Steam can be started without AetherDesk. Preserve a newly seen
            // Lua now, but never replace a non-empty canonical backup here;
            // AetherDesk's richer startup sync owns history/version decisions.
            const fs::path luaDestination = appLuaBackup / (std::to_string(appId) + ".lua");
            if (IsMissingOrEmpty(luaDestination)) {
                const fs::path temporary = luaDestination.string() + ".aether-tmp";
                std::error_code copyEc;
                fs::copy_file(luaEntry.path(), temporary,
                              fs::copy_options::overwrite_existing, copyEc);
                if (!copyEc && fs::exists(luaDestination, copyEc)) {
                    fs::remove(luaDestination, copyEc);
                }
                if (!copyEc) fs::rename(temporary, luaDestination, copyEc);
                if (copyEc) {
                    std::error_code cleanupEc;
                    fs::remove(temporary, cleanupEc);
                } else {
                    ++luaCopied;
                }
            }

            std::set<std::string> filenames;
            for (std::sregex_iterator match(content.begin(), content.end(), pinRegex), end;
                 match != end; ++match) {
                const std::string depot = (*match)[1].str();
                const std::string gid = (*match)[2].str();
                filenames.insert(depot + "_" + gid + ".manifest");
            }
            referenced += filenames.size();

            for (const std::string& filename : filenames) {
                const fs::path source = depotcache / filename;
                if (!fs::is_regular_file(source, ec) || ec) {
                    ++missing;
                    continue;
                }
                const fs::path destination = appLuaBackup / filename;
                if (!IsMissingOrEmpty(destination)) {
                    ++alreadyPresent;
                    continue;
                }

                const fs::path temporary = destination.string() + ".aether-tmp";
                std::error_code copyEc;
                fs::copy_file(source, temporary,
                              fs::copy_options::overwrite_existing, copyEc);
                if (!copyEc && fs::exists(destination, copyEc)) {
                    fs::remove(destination, copyEc);
                }
                if (!copyEc) fs::rename(temporary, destination, copyEc);
                if (copyEc) {
                    std::error_code cleanupEc;
                    fs::remove(temporary, cleanupEc);
                    if (!IsMissingOrEmpty(destination)) {
                        ++alreadyPresent;
                    } else {
                        AC_LOG_DEBUG(kModule, "Cannot backup %s (%s).",
                                     source.string().c_str(), copyEc.message().c_str());
                    }
                } else {
                    ++copied;
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
    if (!g_state.settings.manifestRestoreOnStartup) {
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
        AC_LOG_WARN(kModule, "Restore skipped: cannot create %s (%s).",
                    depotcache.string().c_str(), ec.message().c_str());
        return;
    }
    if (!fs::is_directory(backupRoot, ec) || ec) {
        AC_LOG_INFO(kModule, "No backup root at %s; nothing to restore.",
                    backupRoot.string().c_str());
        return;
    }

    RestoreStats total;
    fs::directory_iterator appIt(backupRoot, ec);
    if (ec) {
        AC_LOG_WARN(kModule, "Cannot list %s (%s).", backupRoot.string().c_str(),
                    ec.message().c_str());
        return;
    }
    for (const auto& appEntry : appIt) {
        std::error_code entryEc;
        if (!appEntry.is_directory(entryEc) || entryEc) continue;
        total += RestoreLuaDirectory(appEntry.path() / "lua", depotcache);
    }

    AC_LOG_INFO(kModule,
                "Restore complete: scanned=%zu restored=%zu already_present=%zu errors=%zu "
                "(%s -> %s).",
                total.scanned, total.restored, total.present, total.errors,
                backupRoot.string().c_str(), depotcache.string().c_str());
}

void RestoreAppOnce(std::uint32_t appId) {
    if (!g_state.settings.manifestRestoreOnStartup) {
        AC_LOG_DEBUG(kModule, "Restore after ACF removal disabled by config; app=%u.", appId);
        return;
    }

    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty()) return;
    const fs::path backupRoot = fs::path(deskData) / "backup";
    const fs::path depotcache = fs::path(g_state.steamInstallPath) / "depotcache";
    std::error_code ec;
    fs::create_directories(depotcache, ec);
    if (ec) return;

    const RestoreStats stats = RestoreLuaDirectory(
        backupRoot / std::to_string(appId) / "lua", depotcache);
    AC_LOG_INFO(kModule,
                "Restore after appmanifest_%u.acf removal: scanned=%zu restored=%zu "
                "already_present=%zu errors=%zu.",
                appId, stats.scanned, stats.restored, stats.present, stats.errors);
}

}  // namespace

void RestoreMissingManifestsAtStartup() {
    if (s_started.exchange(true)) return;  // una sola volta per processo
    try {
        // Il backup viene eseguito prima del restore: se AetherDesk non è
        // stato avviato, Steam può comunque catturare i manifest ancora
        // presenti in depotcache e conservarli per un futuro uninstall.
        std::thread([] {
            try {
                BackupReferencedManifestsAtStartup();
                RestoreAllOnce();
            } catch (const std::exception& e) {
                AC_LOG_ERROR(kModule, "Startup manifest worker failed: %s.", e.what());
            } catch (...) {
                AC_LOG_ERROR(kModule, "Startup manifest worker failed with unknown exception.");
            }
        }).detach();
    } catch (const std::exception& e) {
        AC_LOG_ERROR(kModule, "Cannot start restore worker: %s.", e.what());
    } catch (...) {
        AC_LOG_ERROR(kModule, "Cannot start restore worker (unknown exception).");
    }
}

void RestoreMissingManifestsForApp(std::uint32_t appId) {
    if (appId == 0) return;
    try {
        // Chiamata dal thread DirWatch già dopo il debounce dell'evento ACF;
        // tenerla sincrona evita che Steam possa uscire prima del trasferimento.
        RestoreAppOnce(appId);
    } catch (const std::exception& e) {
        AC_LOG_ERROR(kModule, "Restore for AppID %u failed: %s.", appId, e.what());
    } catch (...) {
        AC_LOG_ERROR(kModule, "Restore for AppID %u failed with unknown exception.", appId);
    }
}

}  // namespace ac::hooks::ManifestRestore
