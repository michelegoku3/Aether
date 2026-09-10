#include "pch.h"
#include "hooks/wire/ManifestRestore.h"

#include <atomic>
#include <cctype>
#include <exception>
#include <filesystem>
#include <string>
#include <thread>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "hooks/wire/BackupIo.h"

namespace ac::hooks::ManifestRestore {
namespace {

namespace fs = std::filesystem;

constexpr const char* kModule = "ManifestRestore";

// Una sola esecuzione per processo di Steam.
std::atomic<bool> s_started{false};

bool HasManifestExtension(const fs::path& path) {
    // Confronto case-insensitive: i provider scrivono ".manifest", ma il
    // filesystem Windows non distingue maiuscole/minuscole.
    std::string ext = path.extension().string();
    if (ext.size() != 9) return false;  // ".manifest"
    for (char& c : ext) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return ext == ".manifest";
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

void RestoreOnce() {
    if (!g_state.settings.manifestRestoreOnStartup) {
        AC_LOG_DEBUG(kModule, "Restore on startup disabled by config; skipping.");
        return;
    }

    const std::string deskData = backup::io::CachedDeskDataDir();
    if (deskData.empty()) {
        // CachedDeskDataDir logga già il motivo (desk_path.cfg mancante).
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

    std::size_t scanned = 0;
    std::size_t restored = 0;
    std::size_t present = 0;
    std::size_t errors = 0;

    fs::directory_iterator appIt(backupRoot, ec);
    if (ec) {
        AC_LOG_WARN(kModule, "Cannot list %s (%s).", backupRoot.string().c_str(),
                    ec.message().c_str());
        return;
    }
    for (const auto& appEntry : appIt) {
        std::error_code entryEc;
        if (!appEntry.is_directory(entryEc) || entryEc) continue;

        // Solo backup/<app_id>/lua/*.manifest (non ricorsivo: history/
        // contiene solo vecchie versioni .lua, mai manifest).
        const fs::path luaDir = appEntry.path() / "lua";
        if (!fs::is_directory(luaDir, entryEc) || entryEc) continue;

        fs::directory_iterator luaIt(luaDir, entryEc);
        if (entryEc) continue;
        for (const auto& fileEntry : luaIt) {
            std::error_code fileEc;
            if (!fileEntry.is_regular_file(fileEc) || fileEc) continue;
            const fs::path src = fileEntry.path();
            if (!HasManifestExtension(src)) continue;
            ++scanned;

            // filename() non può contenere separatori: nessun path traversal.
            const fs::path dest = depotcache / src.filename();
            if (!IsMissingOrEmpty(dest)) {
                ++present;
                continue;
            }
            // copy_options::none fallisce se la destinazione esiste: con un
            // secondo processo Steam in corsa non sovrascriviamo mai nulla.
            fs::copy_file(src, dest, fs::copy_options::none, fileEc);
            if (fileEc) {
                // RACE benigna: il file è comparso nel frattempo (altro
                // processo / Steam stesso). Ricontrolla prima di gridare errore.
                if (!IsMissingOrEmpty(dest)) {
                    ++present;
                } else {
                    ++errors;
                    AC_LOG_DEBUG(kModule, "Cannot restore %s (%s).",
                                 dest.string().c_str(), fileEc.message().c_str());
                }
            } else {
                ++restored;
            }
        }
    }

    AC_LOG_INFO(kModule,
                "Restore complete: scanned=%zu restored=%zu already_present=%zu errors=%zu "
                "(%s -> %s).",
                scanned, restored, present, errors,
                backupRoot.string().c_str(), depotcache.string().c_str());
}

}  // namespace

void RestoreMissingManifestsAtStartup() {
    if (s_started.exchange(true)) return;  // una sola volta per processo
    try {
        // Detached come gli altri worker di startup: la copia è solo I/O
        // locale e non deve mai ritardare l'init di Steam.
        std::thread([] {
            try {
                RestoreOnce();
            } catch (const std::exception& e) {
                AC_LOG_ERROR(kModule, "Restore worker failed: %s.", e.what());
            } catch (...) {
                AC_LOG_ERROR(kModule, "Restore worker failed with unknown exception.");
            }
        }).detach();
    } catch (const std::exception& e) {
        AC_LOG_ERROR(kModule, "Cannot start restore worker: %s.", e.what());
    } catch (...) {
        AC_LOG_ERROR(kModule, "Cannot start restore worker (unknown exception).");
    }
}

}  // namespace ac::hooks::ManifestRestore
