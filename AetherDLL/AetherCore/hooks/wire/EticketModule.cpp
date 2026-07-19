#include "pch.h"
#include "hooks/wire/EticketModule.h"

#include <vector>

#include "core/Constants.h"
#include "credentials/CredentialStore.h"
#include "core/Logger.h"
#include "scripting/LuaData.h"

#include "steam_messages.pb.h"

namespace ac::hooks::EticketModule {
namespace {

constexpr const char* kModule = "Wire.ETicket";
constexpr std::int32_t kNoChange = -1;

}  // namespace

std::int32_t HandleRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientRequestEncryptedAppTicketResponse resp;
    if (!resp.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        AC_LOG_WARN(kModule, "Parse failed.");
        return kNoChange;
    }
    if (resp.eresult() == static_cast<std::int32_t>(constants::kEResultOk)) {
        return kNoChange;
    }
    if (!resp.has_app_id() || !luadata::IsConfigured(resp.app_id())) {
        return kNoChange;
    }

    std::vector<std::uint8_t> ticket = credential::ReadEncryptedTicket(resp.app_id());
    if (ticket.empty()) return kNoChange;

    if (!resp.mutable_encrypted_app_ticket()->ParseFromArray(ticket.data(), static_cast<int>(ticket.size()))) {
        AC_LOG_WARN(kModule, "Stored ETicket parse failed for app %u.", resp.app_id());
        return kNoChange;
    }

    resp.set_eresult(static_cast<std::int32_t>(constants::kEResultOk));
    const std::uint32_t size = static_cast<std::uint32_t>(resp.ByteSizeLong());
    if (size > outCap || !resp.SerializeToArray(out, static_cast<int>(outCap))) {
        AC_LOG_WARN(kModule, "Encode failed for app %u size=%u.", resp.app_id(), size);
        return kNoChange;
    }

    AC_LOG_INFO(kModule, "Patched ETicket response for app %u with %zuB stored ticket.",
                resp.app_id(), ticket.size());
    return static_cast<std::int32_t>(size);
}

}  // namespace ac::hooks::EticketModule
