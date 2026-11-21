#include "pch.h"
#include "security/HubcapKeyCheck.h"

#include <atomic>
#include <ctime>
#include <string>

#include "core/AetherCoreState.h"
#include "core/Logger.h"
#include "core/Workers.h"
#include "diagnostics/StatusWriter.h"
#include "network/RuntimeHttp.h"
#include "security/ProviderCredentials.h"

namespace ac::hubcapkey {
namespace {

constexpr const char* kModule = "HubcapKey";
constexpr const char* kUserStatsUrl = "https://hubcapmanifest.com/api/v1/user/stats";
// Metadata endpoint: the probe never spends generation budget, so a short
// budget is enough and startup is not delayed by a slow answer.
constexpr int kUserStatsTimeoutSec = 5;

// One check per process. The worker is a one-shot, but a second StartCheck
// (late init, tests) must not raise a second warning box.
std::atomic<bool> s_checked{false};

void ShowExpiredNotice(Reason reason) {
    const wchar_t* detail = reason == Reason::AccountInactive
                                ? L"The Hubcap account behind the saved API key can no longer make requests."
                                : L"Hubcap rejected the saved API key: it is invalid, revoked or expired.";
    // Same wording as the AetherDesk modal: the player must know what stops
    // working, wherever the key is replaced.
    const std::wstring text =
        std::wstring(detail) +
        L"\n\nAdding games, manifest generation, Workshop mods, and whatever else that need Hubcap "
        L"will fail until you replace the key in Settings.";
    // No owner window: the box must appear over Steam even when it is raised
    // before Steam's main window exists. It stays until the player clicks OK.
    ::MessageBoxW(nullptr, text.c_str(), L"Aether - Hubcap API Key expired",
                  MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST);
}

void RunCheck(bool showNotice) {
    const auto apiKey = security::ReadHubcapApiKey();
    if (!apiKey) {
        AC_LOG_DEBUG(kModule, "No saved Hubcap key; skipping the startup check.");
        return;
    }

    const http::Response response = http::GetUncheckedWithHeaders(
        kUserStatsUrl, kUserStatsTimeoutSec, {"Authorization: Bearer " + *apiKey}, L"AetherCore/HubcapKeyCheck/1.0");
    const Verdict verdict = Classify(response.status, response.networkError, response.body);

    g_state.hubcapKeyState.store(static_cast<int>(verdict.state));
    g_state.hubcapKeyCheckedEpoch.store(static_cast<std::int64_t>(std::time(nullptr)));
    status::Write();

    AC_LOG_INFO(kModule, "Key check complete state=%s reason=%s network=%d HTTP=%d.", StateName(verdict.state),
                ReasonName(verdict.reason), response.networkError ? 1 : 0, response.status);
    if (verdict.state == State::Inactive) {
        if (!showNotice) {
            AC_LOG_WARN(kModule, "Saved Hubcap key is not active; startup warning suppressed (Steam beta).");
            return;
        }
        AC_LOG_WARN(kModule, "Saved Hubcap key is not active; showing the startup warning.");
        ShowExpiredNotice(verdict.reason);
    }
}

} // namespace

bool StartCheck(bool showNotice) {
    if (s_checked.exchange(true))
        return true;
    const bool started = workers::StartWorker("hubcap-key-check", [showNotice](std::atomic<bool>& stop) {
        (void)stop;
        RunCheck(showNotice);
    });
    if (!started) {
        AC_LOG_WARN(kModule, "Could not start the key-check worker; skipping the startup check.");
        s_checked.store(false);
    }
    return started;
}

} // namespace ac::hubcapkey
