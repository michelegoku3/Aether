#pragma once

#include <string>
#include <mutex>
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

// Why a hook is not installed. The one diagnostic line a hook miss produces
// used to claim "pattern not resolved" for every cause — address collisions,
// missing spec metadata, MinHook failures — which made the message useless
// exactly when it mattered (a hook silently not applied). Each reason is now
// distinct so the log, the diagnostics ring and aetherdll_status.json all say
// what actually happened.
enum class MissReason {
    /// The per-build pattern table had no match for this hook.
    PatternUnresolved,
    /// Two different hooks resolved to the same address: the pattern metadata
    /// is inconsistent, so hooking would corrupt the wrong function.
    AddressCollision,
    /// Required metadata (e.g. an IPC spec entry) is missing, so the handler
    /// was deliberately disabled.
    MetadataUnavailable,
    /// Two handlers map to the same IPC (interface, hash) pair: the second one
    /// is disabled to keep dispatch deterministic.
    HandlerCollision,
    /// Nothing to do in this process/module (feature not applicable).
    RuntimeNotApplicable,
    /// MinHook refused to create the hook.
    InstallFailed,
};

const char* MissReasonText(MissReason reason);

/// One missed hook: the name, why it is not installed, and — for the reasons
/// where a counterpart exists — which hook it lost to.
///
/// `detail` is not decoration: "address collision with another hook" says the
/// hook was skipped on purpose but not which pattern metadata is wrong, and
/// that is exactly what a user reporting a missing feature has to know. The
/// log line keeps the short reason; status.json carries the detail, because it
/// is the only place a miss survives after the log rotates.
struct MissedHook {
    std::string name;
    MissReason reason = MissReason::PatternUnresolved;
    std::string detail;
};

/// Single definition of how a missed hook is rendered, shared by the log, the
/// diagnostics ring and status.json so the three can never disagree.
std::string MissedHookText(const std::string& name, MissReason reason,
                           const std::string& detail = {});

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

    // Records that a hook is not installed, and why. Feeds the status report;
    // does not throw. Reason is mandatory: the caller knows the cause, and a
    // default would silently re-label collisions as pattern misses.
    // `detail` names the counterpart when the reason has one (the hook that
    // already owns the address, the IPC handler that already owns the pair).
    void RecordMissed(const std::string& name, MissReason reason,
                      const std::string& detail = {});

    // Creates and enables every queued hook. Returns true if enabling
    // succeeded. Individual creation failures are logged and counted, not
    // fatal (graceful degradation).
    bool InstallAll();

    // Disables and tears down all hooks. Safe to call multiple times.
    bool UninstallAll();

    // ---- Status accessors (consumed by StatusWriter) ----------------------
    struct StatusSnapshot {
        std::vector<std::string> installed;
        std::vector<MissedHook> missed;
    };
    StatusSnapshot Snapshot() const;

    // Resolve target via PatternEngine, cast trampoline/detour, and register.
    // Returns false (and records a miss) when the pattern cannot be resolved,
    // so callers can branch without repeating the boilerplate.
    template <typename Fn>
    bool TryHook(const std::string& name, const std::string& module, HMODULE hModule,
                 Fn& original, Fn detour);

private:
    mutable std::mutex mutex_;
    std::vector<HookInfo> hooks_;
    std::vector<std::string> installed_;
    std::vector<MissedHook> missed_;
    int installedCount_ = 0;
};

}  // namespace ac

// Template body needs ResolveAddress; included after the class so the header
// stays self-contained.
#include "core/HookManager.inl"
