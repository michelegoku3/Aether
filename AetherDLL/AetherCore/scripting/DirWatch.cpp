#include "pch.h"
#include "scripting/DirWatch.h"

#include <algorithm>
#include <atomic>
#include <filesystem>
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
#include "utils/SmartIdLog.h"

namespace fs = std::filesystem;

namespace ac::dirwatch {
namespace {

constexpr const char* kModule = "DirWatch";
constexpr DWORD kBufferBytes = 64 * 1024;
constexpr DWORD kDebounceMs = 500;

// Module-owned lifecycle service. This is intentionally not in AetherCoreState:
// it is not cross-module domain data, but the private thread/control block for
// the directory watcher. All shared Lua data mutated by the watcher lives in
// g_state.lua and is accessed through LuaData.
struct WatcherService {
    std::atomic<bool> running{false};
    std::thread thread;
    std::vector<std::string> dirs;
};
WatcherService s_watch;

// Converts a UTF-16 file name from ReadDirectoryChangesW to UTF-8. This keeps
// non-ASCII .lua names usable instead of replacing them with '?'.
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

bool IsLuaPath(const fs::path& path) {
    return path.extension() == ".lua";
}

std::string NormalizedPath(const fs::path& path) {
    return path.lexically_normal().make_preferred().string();
}

// One watched directory: handle + overlapped read + scratch buffer.
struct Slot {
    std::string path;
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
        // FILE_ACTION_RENAMED_OLD_NAME and FILE_ACTION_RENAMED_NEW_NAME are
        // included so that renames are treated as remove + add respectively,
        // keeping the Lua hot-reload view consistent with the filesystem.
        BOOL ok = ReadDirectoryChangesW(dir, buffer.data(), kBufferBytes, FALSE,
                                        FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
                                        &n, &ov, nullptr);
        return ok || GetLastError() == ERROR_IO_PENDING;
    }

    // Drains one completed read into acc (full path -> last action) and re-arms.
    // Sets overflowed to true when the buffer was too small, signalling the
    // caller to do a full directory rescan.
    bool Harvest(std::unordered_map<std::string, DWORD>& acc, std::vector<std::string>& order,
                 bool& overflowed) {
        overflowed = false;
        DWORD n = 0;
        // Since WaitForMultipleObjects already signalled completion, the
        // non-blocking call should succeed. If n == 0 the buffer overflowed
        // and we lost events — signal the caller for a full rescan.
        if (!GetOverlappedResult(dir, &ov, &n, FALSE) || n == 0) {
            overflowed = (n == 0);
            Arm();
            return true;
        }
        const auto* rec = reinterpret_cast<const FILE_NOTIFY_INFORMATION*>(buffer.data());
        while (rec) {
            // Rename-from is treated as removal; rename-to as addition.
            if (rec->Action == FILE_ACTION_ADDED || rec->Action == FILE_ACTION_MODIFIED ||
                rec->Action == FILE_ACTION_REMOVED ||
                rec->Action == FILE_ACTION_RENAMED_NEW_NAME ||
                rec->Action == FILE_ACTION_RENAMED_OLD_NAME) {
                std::wstring_view wname(rec->FileName, rec->FileNameLength / sizeof(wchar_t));
                if (wname.size() >= 4 && wname.substr(wname.size() - 4) == L".lua") {
                    std::string name = WideToUtf8(wname);
                    if (name.empty()) {
                        AC_LOG_WARN(kModule, "Could not convert changed Lua filename to UTF-8.");
                    } else {
                        std::string full = NormalizedPath(fs::path(path) / fs::path(name));
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

std::unordered_set<std::string> EnumerateLuaFiles(const std::string& dir) {
    std::unordered_set<std::string> out;
    try {
        for (const auto& entry : fs::recursive_directory_iterator(dir)) {
            if (!entry.is_regular_file()) continue;
            if (!IsLuaPath(entry.path())) continue;
            out.insert(NormalizedPath(entry.path()));
        }
    } catch (...) {
        AC_LOG_WARN(kModule, "Enumerating Lua files in '%s' failed.", dir.c_str());
    }
    return out;
}

// Performs a full rescan after an overflow. Unlike the old recovery path, this
// detects both additions/modifications and removals by diffing against the
// slot's known file set.
void FullRescan(Slot& slot, std::unordered_map<std::string, DWORD>& acc,
                std::vector<std::string>& order) {
    AC_LOG_WARN(kModule, "Buffer overflow in '%s'; performing full rescan.", slot.path.c_str());
    diag::Record("dirwatch_overflow", slot.path);

    std::unordered_set<std::string> current = EnumerateLuaFiles(slot.path);
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
    slot.knownFiles = std::move(current);
}

// Applies a debounced batch of file changes and refreshes licenses once.
void ApplyChanges(const std::unordered_map<std::string, DWORD>& acc,
                  const std::vector<std::string>& order) {
    if (order.empty()) return;
    AC_LOG_INFO(kModule, "Processing %zu Lua change(s).", order.size());
    diag::Record("lua_hot_reload", std::to_string(order.size()) + " change(s)");
    for (const std::string& path : order) {
        DWORD action = acc.at(path);
        // Rename-old is treated as removal; rename-new and additions trigger a re-parse.
        if (action == FILE_ACTION_REMOVED || action == FILE_ACTION_RENAMED_OLD_NAME) {
            script::UnloadFile(path);
        } else {
            script::ParseFile(path);
        }
    }
    hooks::LicenseManager::NotifyLicenseChanged();
    logutil::ResetAllIdLogSessions();
    log::ResetDedup();
    pipewatch::ResetSessionTracking();
    status::Write();
    AC_LOG_INFO(kModule, "Hot-reload refresh complete.");
}

void Run() {
    std::vector<Slot> slots(s_watch.dirs.size());
    std::vector<HANDLE> events;
    for (std::size_t i = 0; i < s_watch.dirs.size(); ++i) {
        slots[i].path = s_watch.dirs[i];
        if (slots[i].Open()) {
            slots[i].knownFiles = EnumerateLuaFiles(slots[i].path);
            events.push_back(slots[i].event);
            AC_LOG_INFO(kModule, "Watching '%s'.", s_watch.dirs[i].c_str());
        }
    }
    if (events.empty()) {
        AC_LOG_WARN(kModule, "No directories could be watched; watcher exiting.");
        return;
    }

    // Win32 caps the wait at MAXIMUM_WAIT_OBJECTS handles.
    DWORD count = static_cast<DWORD>(std::min<std::size_t>(events.size(), MAXIMUM_WAIT_OBJECTS));

    while (s_watch.running.load()) {
        DWORD wr = WaitForMultipleObjects(count, events.data(), FALSE, 1000);
        if (!s_watch.running.load()) break;
        if (wr < WAIT_OBJECT_0 || wr >= WAIT_OBJECT_0 + count) continue;  // timeout/error

        std::unordered_map<std::string, DWORD> acc;
        std::vector<std::string> order;
        bool overflowed = false;
        slots[wr - WAIT_OBJECT_0].Harvest(acc, order, overflowed);
        if (overflowed) FullRescan(slots[wr - WAIT_OBJECT_0], acc, order);

        // Debounce: keep draining until a quiet window elapses.
        while (s_watch.running.load()) {
            DWORD dr = WaitForMultipleObjects(count, events.data(), FALSE, kDebounceMs);
            if (!s_watch.running.load() || dr < WAIT_OBJECT_0 || dr >= WAIT_OBJECT_0 + count) break;
            bool ovf = false;
            slots[dr - WAIT_OBJECT_0].Harvest(acc, order, ovf);
            if (ovf) FullRescan(slots[dr - WAIT_OBJECT_0], acc, order);
        }
        ApplyChanges(acc, order);
    }

    for (auto& s : slots) s.Close();
    AC_LOG_INFO(kModule, "Stopped.");
}

}  // namespace

void Start(const std::vector<std::string>& directories) {
    if (directories.empty()) {
        AC_LOG_WARN(kModule, "No directories configured; watcher not started.");
        return;
    }
    if (s_watch.running.exchange(true)) {
        AC_LOG_WARN(kModule, "Already running.");
        return;
    }
    s_watch.dirs.clear();
    for (const std::string& d : directories) {
        try {
            s_watch.dirs.push_back(fs::path(d).lexically_normal().make_preferred().string());
        } catch (...) {
            s_watch.dirs.push_back(d);
        }
    }
    s_watch.thread = std::thread(Run);
}

void Stop() {
    if (!s_watch.running.exchange(false)) return;
    if (s_watch.thread.joinable()) s_watch.thread.join();
}

}  // namespace ac::dirwatch
