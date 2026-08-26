#pragma once

// IClientUtils IPC handlers: GetAppID (restore the real app id for AetherOnline
// games masked as 480) and GetAPICallResult (inject k_EResultOK for a pending
// encrypted-app-ticket request).
namespace ac::hooks::CmdUtils {

void Register();

}  // namespace ac::hooks::CmdUtils
