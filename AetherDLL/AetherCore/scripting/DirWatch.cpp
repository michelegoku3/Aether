#include "pch.h"
#include "scripting/DirWatch.h"

#include <algorithm>
#include <atomic>
#include <charconv>
#include <cctype>
#include <cstdint>
#include <filesystem>
#include <string_view>
#include <system_error>
#include <optional>
#include <string>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "core/Logger.h"
#include "scripting/ScriptEngine.h"
#include "diagnostics/StatusWriter.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/license/LicenseManager.h"
#include "hooks/wire/ManifestRestore.h"
#include "utils/SmartIdLog.h"

namespace fs = std::filesystem;

namespace ac::dirwatch {
namespace {

constexpr const char* kModule = "DirWatch";
constexpr DWORD kBufferBytes = 64 * 1024;
constexpr DWORD kDebounceMs = 500;

// Module-owned lifecycle service. This is intentionally not in AetherCoreState:
// it is the private thread/control block for the directory watcher. Lua data
// mutated by the watcher lives in g_state.lua and is protected by LuaData.
struct WatcherService {
    std::atomic<bool> running{false};
    std::thread thread;
    std::vector<std::string> luaDirs;
    std::vector<std::string> acfDirs;
};
WatcherService s_watch;

std::string WideToUtf8(std::wstring_view w) {
    if (w.empty()) return {};
    int needed = WideCharToMultiByte(CP_UTF8, 0, w.data(), static_cast<int>(w.size()),
                                     nullptr, 0, nullptr, nullptr);
    if (needed <= 0) return {};
    std::string out(static_cast<std::size_t>(needed), '\0');
    WideCharToMultiByte(CP_UTF8, 0, w.data(), static_cast<int>(w.size()), out.data(),
                        needed, nullptr, nullptr);
    return out;
}

bool HasExtension(const fs::path& path, const char* wanted) {
    std::string ext = path.extension().string();
    std::string expected = wanted;
    if (ext.size() != expected.size()) return false;
    for (char& c : ext) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    for (char& c : expected) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return ext == expected;
}

bool IsLuaPath(const fs::path& path) {
    return HasExtension(path, ".lua");
}

bool IsAppManifestPath(const fs::path& path) {
    if (!HasExtension(path, ".acf")) return false;
    std::string stem = path.stem().string();
    constexpr std::string_view prefix = "appmanifest_";
    if (stem.size() <= prefix.size()) return false;
    for (std::size_t i = 0; i < prefix.size(); ++i) {
        const char actual = static_cast<char>(std::tolower(static_cast<unsigned char>(stem[i])));
        if (actual != prefix[i]) return false;
    }
    for (std::size_t i = prefix.size(); i < stem.size(); ++i) {
        if (stem[i] < '0' || stem[i] > '9') return false;
    }
    return true;
}

std::optional<std::uint32_t> AppIdFromManifestPath(const fs::path& path) {
    if (!IsAppManifestPath(path)) return std::nullopt;
    const std::string stem = path.stem().string();
    constexpr std::size_t prefixLength = sizeof("appmanifest_") - 1;
    std::uint32_t appId = 0;
    const char* begin = stem.data() + prefixLength;
    const char* end = stem.data() + stem.size();
    const auto result = std::from_chars(begin, end, appId);
    if (result.ec != std::errc{} || result.ptr != end || appId == 0) return std::nullopt;
    return appId;
}

std::string NormalizedPath(const fs::path& path) {
    return path.lexically_normal().make_preferred().string();
}

struct Slot {
    std::string path;
    bool acfOnly = false;
    HANDLE dir = INVALID_HANDLE_VALUE;
    HANDLE event = nullptr;
    OVERLAPPED ov{};
    std::vector<std::uint8_t> buffer = std::vector<std::uint8_t>(kBufferBytes);
    std::unordered_set<std::string> knownFiles;

    bool Open() {
        event = CreateEventA(nullptr, FALSE, FALSE, nullptr);
        if (!event) return false;
        ov = {};
        ov.hEvent = event;
        dir = CreateFileA(path.c_str(), FILE_LIST_DIRECTORY,
                          FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                          OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED, nullptr);
        if (dir == INVALID_HANDLE_VALUE) {
            CloseHandle(event);
            event = nullptr;
            return false;
        }
        return Arm();
    }

    bool Arm() {
        DWORD n = 0;
        BOOL ok = ReadDirectoryChangesW(dir, buffer.data(), kBufferBytes, FALSE,
                                        FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
                                        &n, &ov, nullptr);
        return ok || GetLastError() == ERROR_IO_PENDING;
    }

    bool AcceptName(const fs::path& name) const {
        return acfOnly ? IsAppManifestPath(name) : IsLuaPath(name);
    }

    bool Harvest(std::unordered_map<std::string, DWORD>& acc, std::vector<std::string>& order,
                 bool& overflowed) {
        overflowed = false;
        DWORD n = 0;
        if (!GetOverlappedResult(dir, &ov, &n, FALSE) || n == 0) {
            overflowed = (n == 0);
            Arm();
            return true;
        }
        const auto* rec = reinterpret_cast<const FILE_NOTIFY_INFORMATION*>(buffer.data());
        while (rec) {
            if (rec->Action == FILE_ACTION_ADDED || rec->Action == FILE_ACTION_MODIFIED ||
                rec->Action == FILE_ACTION_REMOVED ||
                rec->Action == FILE_ACTION_RENAMED_NEW_NAME ||
                rec->Action == FILE_ACTION_RENAMED_OLD_NAME) {
                std::wstring_view wname(rec->FileName, rec->FileNameLength / sizeof(wchar_t));
                const fs::path relativeName(WideToUtf8(wname));
                if (!relativeName.empty() && AcceptName(relativeName)) {
                    const std::string name = WideToUtf8(wname);
                    if (name.empty()) {
                        AC_LOG_WARN(kModule, "Could not convert changed filename to UTF-8.");
                    } else {
                        const std::string full = NormalizedPath(fs::path(path) / fs::path(name));
                        if (!acc.count(full)) order.push_back(full);
                        acc[full] = rec->Action;
                        if (rec->Action == FILE_ACTION_REMOVED ||
                            rec->Action == FILE_ACTION_RENAMED_OLD_NAME) {
                            knownFiles.erase(full);
                        } else {
                            knownFiles.insert(full);
                        }
                    }
                }
            }
            if (!rec->NextEntryOffset) break;
            rec = reinterpret_cast<const FILE_NOTIFY_INFORMATION*>(
                reinterpret_cast<const std::uint8_t*>(rec) + rec->NextEntryOffset);
        }
        return Arm();
    }

    void Close() {
        if (dir != INVALID_HANDLE_VALUE) { CloseHandle(dir); dir = INVALID_HANDLE_VALUE; }
        if (event) { CloseHandle(event); event = nullptr; }
    }
};

std::unordered_set<std::string> EnumerateWatchedFiles(const Slot& slot) {
    std::unordered_set<std::string> out;
    std::error_code ec;
    if (slot.acfOnly) {
        for (const auto& entry : fs::directory_iterator(slot.path, ec)) {
            if (ec) break;
            if (entry.is_regular_file(ec) && !ec && IsAppManifestPath(entry.path())) {
                out.insert(NormalizedPath(entry.path()));
            }
        }
    } else {
        for (const auto& entry : fs::recursive_directory_iterator(slot.path, ec)) {
            if (ec) break;
            if (entry.is_regular_file(ec) && !ec && IsLuaPath(entry.path())) {
                out.insert(NormalizedPath(entry.path()));
            }
        }
    }
    return out;
}

void FullRescan(Slot& slot, std::unordered_map<std::string, DWORD>& acc,
                std::vector<std::string>& order) {
    AC_LOG_WARN(kModule, "Buffer overflow in '%s'; performing full rescan.", slot.path.c_str());
    diag::Record("dirwatch_overflow", slot.path);

    const std::unordered_set<std::string> current = EnumerateWatchedFiles(slot);
    for (const std::string& path : current) {
        if (!acc.count(path)) order.push_back(path);
        acc[path] = FILE_ACTION_ADDED;
    }
    for (const std::string& oldPath : slot.knownFiles) {
        if (!current.count(oldPath)) {
            if (!acc.count(oldPath)) order.push_back(oldPath);
            acc[oldPath] = FILE_ACTION_REMOVED;
        }
    }
    slot.knownFiles = current;
}

void ApplyChanges(const std::unordered_map<std::string, DWORD>& acc,
                  const std::vector<std::string>& order) {
    if (order.empty()) return;
    AC_LOG_INFO(kModule, "Processing %zu watched change(s).", order.size());

    bool luaChanged = false;
    std::vector<std::uint32_t> removedApps;
    for (const std::string& path : order) {
        const DWORD action = acc.at(path);
        if (IsAppManifestPath(fs::path(path))) {
            if (action == FILE_ACTION_REMOVED || action == FILE_ACTION_RENAMED_OLD_NAME) {
                if (const auto appId = AppIdFromManifestPath(fs::path(path))) {
                    removedApps.push_back(*appId);
                }
            }
            continue;
        }

        luaChanged = true;
        if (action == FILE_ACTION_REMOVED || action == FILE_ACTION_RENAMED_OLD_NAME) {
            script::UnloadFile(path);
        } else {
            script::ParseFile(path);
        }
    }

    if (luaChanged) {
        diag::Record("lua_hot_reload", std::to_string(order.size()) + " change(s)");
        hooks::LicenseManager::NotifyLicenseChanged();
        logutil::ResetAllIdLogSessions();
        log::ResetDedup();
        pipewatch::ResetSessionTracking();
        status::Write();
        AC_LOG_INFO(kModule, "Lua hot-reload refresh complete.");
    }

    std::sort(removedApps.begin(), removedApps.end());
    removedApps.erase(std::unique(removedApps.begin(), removedApps.end()), removedApps.end());
    for (const std::uint32_t appId : removedApps) {
        AC_LOG_INFO(kModule,
                    "Detected removal of Steam appmanifest_%u.acf; restoring backed-up manifests.",
                    appId);
        hooks::ManifestRestore::RestoreMissingManifestsForApp(appId);
    }
}

void Run() {
    std::vector<Slot> slots;
    slots.reserve(s_watch.luaDirs.size() + s_watch.acfDirs.size());
    for (const std::string& dir : s_watch.luaDirs) slots.push_back(Slot{dir, false});
    for (const std::string& dir : s_watch.acfDirs) slots.push_back(Slot{dir, true});

    std::vector<HANDLE> events;
    std::vector<std::size_t> eventSlots;
    for (std::size_t i = 0; i < slots.size(); ++i) {
        if (slots[i].Open()) {
            slots[i].knownFiles = EnumerateWatchedFiles(slots[i]);
            events.push_back(slots[i].event);
            eventSlots.push_back(i);
            AC_LOG_INFO(kModule, "Watching '%s' (%s).", slots[i].path.c_str(),
                        slots[i].acfOnly ? "Steam app manifests" : "Lua files");
        }
    }
    if (events.empty()) {
        AC_LOG_WARN(kModule, "No directories could be watched; watcher exiting.");
        return;
    }

    // Win32 caps the wait at MAXIMUM_WAIT_OBJECTS handles.
    const DWORD count = static_cast<DWORD>(std::min<std::size_t>(events.size(), MAXIMUM_WAIT_OBJECTS));

    while (s_watch.running.load()) {
        const DWORD wr = WaitForMultipleObjects(count, events.data(), FALSE, 1000);
        if (!s_watch.running.load()) break;
        if (wr < WAIT_OBJECT_0 || wr >= WAIT_OBJECT_0 + count) continue;

        std::unordered_map<std::string, DWORD> acc;
        std::vector<std::string> order;
        bool overflowed = false;
        Slot& first = slots[eventSlots[wr - WAIT_OBJECT_0]];
        first.Harvest(acc, order, overflowed);
        if (overflowed) FullRescan(first, acc, order);

        // Debounce: keep draining until a quiet window elapses.
        while (s_watch.running.load()) {
            const DWORD dr = WaitForMultipleObjects(count, events.data(), FALSE, kDebounceMs);
            if (!s_watch.running.load() || dr < WAIT_OBJECT_0 || dr >= WAIT_OBJECT_0 + count) break;
            bool ovf = false;
            Slot& next = slots[eventSlots[dr - WAIT_OBJECT_0]];
            next.Harvest(acc, order, ovf);
            if (ovf) FullRescan(next, acc, order);
        }
        ApplyChanges(acc, order);
    }

    for (auto& slot : slots) slot.Close();
    AC_LOG_INFO(kModule, "Stopped.");
}

std::string NormalizeDirectory(const std::string& directory) {
    try {
        return fs::path(directory).lexically_normal().make_preferred().string();
    } catch (...) {
        return directory;
    }
}

}  // namespace

void Start(const std::vector<std::string>& directories) {
    Start(directories, {});
}

void Start(const std::vector<std::string>& directories,
           const std::vector<std::string>& acfDirectories) {
    if (directories.empty() && acfDirectories.empty()) {
        AC_LOG_WARN(kModule, "No directories configured; watcher not started.");
        return;
    }
    if (s_watch.running.exchange(true)) {
        AC_LOG_WARN(kModule, "Already running.");
        return;
    }
    s_watch.luaDirs.clear();
    s_watch.acfDirs.clear();
    for (const std::string& directory : directories) {
        s_watch.luaDirs.push_back(NormalizeDirectory(directory));
    }
    for (const std::string& directory : acfDirectories) {
        s_watch.acfDirs.push_back(NormalizeDirectory(directory));
    }
    s_watch.thread = std::thread(Run);
}

void Stop() {
    if (!s_watch.running.exchange(false)) return;
    if (s_watch.thread.joinable()) s_watch.thread.join();
}

}  // namespace ac::dirwatch
