#include "pch.h"
#include "network/ManifestIndex.h"

#include <mutex>
#include <unordered_map>

namespace ac::manifestfetch::index {
namespace {

std::mutex s_indexMutex;
std::unordered_map<std::string, std::filesystem::path> s_backupIndex; // filename -> backup lua dir
bool s_backupIndexBuilt = false;

// One full walk per session. Merges (never overwrites): upserts from backup
// writes that landed before the first lookup are preserved.
void RebuildLocked(const std::filesystem::path& backupRoot) {
    namespace fs = std::filesystem;
    std::error_code ec;
    for (fs::directory_iterator it(backupRoot, ec), end; !ec && it != end; it.increment(ec)) {
        if (!it->is_directory(ec) || ec)
            continue;
        const fs::path luaDir = it->path() / "lua";
        for (fs::directory_iterator jt(luaDir, ec), jend; !ec && jt != jend; jt.increment(ec)) {
            std::error_code fileEc;
            if (!jt->is_regular_file(fileEc) || fileEc)
                continue;
            if (jt->path().extension() != ".manifest")
                continue;
            s_backupIndex.try_emplace(jt->path().filename().string(), luaDir);
        }
    }
    s_backupIndexBuilt = true;
}

bool Usable(const std::filesystem::path& path) {
    namespace fs = std::filesystem;
    std::error_code ec;
    if (!fs::is_regular_file(path, ec) || ec)
        return false;
    const auto size = fs::file_size(path, ec);
    return !ec && size > 0;
}

} // namespace

std::optional<std::filesystem::path> FindInBackup(const std::filesystem::path& backupRoot,
                                                  const std::string& filename) {
    std::lock_guard<std::mutex> lock(s_indexMutex);
    if (!s_backupIndexBuilt)
        RebuildLocked(backupRoot);
    const auto it = s_backupIndex.find(filename);
    if (it == s_backupIndex.end())
        return std::nullopt;
    std::filesystem::path candidate = it->second / filename;
    if (!Usable(candidate)) {
        // Stale entry (user cleanup, Desk prune): drop it.
        s_backupIndex.erase(it);
        return std::nullopt;
    }
    return candidate;
}

void NoteFileAdded(const std::string& filename, const std::filesystem::path& backupLuaDir) {
    std::lock_guard<std::mutex> lock(s_indexMutex);
    s_backupIndex[filename] = backupLuaDir;
}

} // namespace ac::manifestfetch::index
