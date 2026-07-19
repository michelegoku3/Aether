#include "pch.h"
#include "hooks/ipc/CmdUtils.h"

#include <cstring>
#include <iterator>

#include "hooks/ipc/CmdUser.h"
#include "core/Constants.h"
#include "hooks/ipc/IPCBus.h"
#include "core/Logger.h"
#include "hooks/ipc/PipeWatch.h"
#include "hooks/ipc/SteamCapture.h"

namespace ac::hooks::CmdUtils {
namespace {

constexpr const char* kModule = "IPC.Utils";
using namespace ac::constants;

// IClientUtils::GetAPICallResult request args.
struct GetAPICallResultRequest {
    std::uint64_t hSteamAPICall;     // +0
    std::uint32_t cubCallback;       // +8
    std::uint32_t iCallbackExpected; // +12
};

// IClientUtils::GetAppID
//   SpawnProcess rewrites the GameID to 480 for OnlineFix games, so steamclient
//   returns 480 here. Restore the real app id in the reply.
void GetAppID(steam::CSteamPipeClient* pipe, steam::CUtlBuffer*, steam::CUtlBuffer* pWrite) {
    const steam::AppId realAppId = pipewatch::AppIdForPipe(pipe);
    if (!realAppId || !pWrite || pWrite->TellPut() < 5) return;

    steam::AppId current = 0;
    std::memcpy(&current, pWrite->Base() + 1, sizeof(current));
    if (current == realAppId) return;

    std::memcpy(pWrite->Base() + 1, &realAppId, sizeof(realAppId));
    AC_LOG_DEBUG_ONCE(kModule, "GetAppID: %u -> %u", current, realAppId);
}

// Injects k_EResultOK for a recorded encrypted-app-ticket async call.
// Reply layout: [tag][success=1][EResult m_eResult][trailing 0].
bool InjectEncryptedAppTicketResult(steam::CUtlBuffer* pWrite, std::uint64_t asyncCall) {
    constexpr std::int32_t kEResultBytes = 4;
    constexpr std::int32_t total = 1 + 1 + kEResultBytes + 1;
    if (!pWrite || !pWrite->Base() || pWrite->TellPut() < total) return false;

    // Validate capacity before claiming. Once the fixed-size response passes
    // this check, writing it cannot fail; a short-buffer retry therefore keeps
    // the pending correlation intact.
    const auto appId = CmdUser::ClaimETicketAsyncCall(asyncCall);
    if (!appId) return false;

    std::uint8_t* base = pWrite->Base();
    base[0] = kIpcReplyTag;
    base[1] = 1;
    std::uint32_t result = kEResultOk;
    std::memcpy(base + 2, &result, sizeof(result));
    base[2 + kEResultBytes] = 0;

    AC_LOG_DEBUG_ONCE(kModule, "GetAPICallResult: EncryptedAppTicketResponse OK (app %u).", *appId);
    return true;
}

// IClientUtils::GetAPICallResult
//   Only the encrypted-app-ticket response is handled. Achievement/user-stats
//   callbacks are intentionally NOT processed (achievement code is excluded).
void GetAPICallResult(steam::CSteamPipeClient*, steam::CUtlBuffer* pRead,
                      steam::CUtlBuffer* pWrite) {
    if (!pRead || !pRead->Base() ||
        pRead->TellPut() < static_cast<std::int32_t>(kIpcArgsOffset + sizeof(GetAPICallResultRequest))) {
        return;
    }
    GetAPICallResultRequest req{};
    std::memcpy(&req, pRead->Base() + kIpcArgsOffset, sizeof(req));

    if (req.iCallbackExpected == kCallbackEncryptedAppTicketResponse) {
        InjectEncryptedAppTicketResult(pWrite, req.hSteamAPICall);
    }
}

const IpcHandlerEntry kEntries[] = {
    {ipc_iface::kClientUtils, ipc_hash::kClientUtils_GetAppID,
     "IClientUtils::GetAppID", GetAppID},
    {ipc_iface::kClientUtils, ipc_hash::kClientUtils_GetAPICallResult,
     "IClientUtils::GetAPICallResult", GetAPICallResult},
};

}  // namespace

void Register() {
    RegisterIpcHandlers(kEntries, std::size(kEntries));
}

}  // namespace ac::hooks::CmdUtils
