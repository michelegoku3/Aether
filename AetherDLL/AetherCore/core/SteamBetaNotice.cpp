#include "pch.h"
#include "core/SteamBetaNotice.h"

#include <atomic>
#include <string>

#include "core/Logger.h"
#include "core/Workers.h"

namespace ac::steambeta {
namespace {

constexpr const char* kModule = "SteamBeta";

// One notice per process: a late init or a test must not stack a second box.
std::atomic<bool> s_notified{false};

void ShowNotice(const std::string& branch) {
    AC_LOG_WARN(kModule, "Steam client beta (%s) is not supported; showing the startup notice.", branch.c_str());
    // Same delivery as the Hubcap key notice: an ownerless topmost box, so it
    // lands over Steam even before the main window exists and stays until the
    // player acknowledges it. The steps mirror Steam's own Settings path.
    const std::wstring text =
        L"You are running the Steam client beta, Aether supports the stable Steam client only.\n\n"
        L"Switch back to stable: Steam > Settings > Interface > Client Beta Participation > choose \"No beta "
        L"chosen\", then restart Steam.";
    ::MessageBoxW(nullptr, text.c_str(), L"Aether - Steam Beta detected",
                  MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST);
}

} // namespace

void StartNotice(const std::string& branch) {
    if (branch.empty() || s_notified.exchange(true))
        return;
    const bool started = workers::StartWorker("steam-beta-notice", [branch](std::atomic<bool>& stop) {
        (void)stop;
        ShowNotice(branch);
    });
    if (!started) {
        AC_LOG_WARN(kModule, "Could not start the beta-notice worker; skipping the startup notice.");
        s_notified.store(false);
    }
}

} // namespace ac::steambeta
