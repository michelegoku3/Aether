#pragma once

#include <string>
#include <vector>

// ---------------------------------------------------------------------------
// Thin wrapper around MinHook.
//
// Improvements over LumaCore's macro-based hook system (DOCS_TODO 11 #2):
//   * No macros: every hook is registered with a plain, debuggable call.
//   * A TryHook<> template removes the repeated "resolve + cast + register"
//     boilerplate each module used to duplicate.
//   * Install/miss bookkeeping is built in, feeding StatusWriter (Phase 1.4)
//     without a second tracking system.
// ---------------------------------------------------------------------------
namespace ac {

struct HookInfo {
    std::string name;
    void* target = nullptr;     // Address of the original function in memory
    void** original = nullptr;  // Out: trampoline to call the original
    void* detour = nullptr;     // Our replacement function
    bool created = false;
};

class HookManager {
public:
    // Queues a hook for later installation. Prefer TryHook() below.
    void RegisterHook(const std::string& name, void* target, void** original, void* detour);

    // Records that a hook could not be installed because its address could not
    // be resolved. Feeds the status report; does not throw.
    void RecordMissed(const std::string& name);

    // Creates and enables every queued hook. Returns true if enabling
    // succeeded. Individual creation failures are logged and counted, not
    // fatal (graceful degradation).
    bool InstallAll();

    // Disables and tears down all hooks. Safe to call multiple times.
    bool UninstallAll();

    // ---- Status accessors (consumed by StatusWriter) ----------------------
    int InstalledCount() const { return installedCount_; }
    const std::vector<std::string>& InstalledHooks() const { return installed_; }
    const std::vector<std::string>& MissedHooks() const { return missed_; }

    // Resolve target via PatternEngine, cast trampoline/detour, and register.
    // Returns false (and records a miss) when the pattern cannot be resolved,
    // so callers can branch without repeating the boilerplate.
    template <typename Fn>
    bool TryHook(const std::string& name, const std::string& module, HMODULE hModule,
                 Fn& original, Fn detour);

private:
    std::vector<HookInfo> hooks_;
    std::vector<std::string> installed_;
    std::vector<std::string> missed_;
    int installedCount_ = 0;
};

}  // namespace ac

// Template body needs ResolveAddress; included after the class so the header
// stays self-contained.
#include "core/HookManager.inl"
