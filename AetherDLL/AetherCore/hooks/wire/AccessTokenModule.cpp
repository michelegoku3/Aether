#include "pch.h"
#include "hooks/wire/AccessTokenModule.h"

#include <string>

#include "scripting/LuaData.h"
#include "core/Logger.h"

#include "steam_messages.pb.h"

namespace ac::hooks::AccessToken {
namespace {
constexpr const char* kModule = "Wire.PICS";
constexpr std::int32_t kNoChange = -1;
}  // namespace

std::int32_t HandleSend(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap) {
    CMsgClientPICSProductInfoRequest req;
    if (!req.ParseFromArray(frame.body, static_cast<int>(frame.bodyLen))) {
        return kNoChange;
    }

    // Only rewrite if at least one requested app has a configured token.
    bool needsPatch = false;
    for (const auto& app : req.apps()) {
        if (luadata::IsConfigured(app.appid()) && luadata::AccessToken(app.appid()) != 0) {
            needsPatch = true;
            break;
        }
    }
    if (!needsPatch) return kNoChange;

    int injected = 0;
    for (auto& app : *req.mutable_apps()) {
        std::uint64_t token = luadata::AccessToken(app.appid());
        if (luadata::IsConfigured(app.appid()) && token != 0) {
            app.set_access_token(token);
            ++injected;
        }
    }

    const std::uint32_t size = static_cast<std::uint32_t>(req.ByteSizeLong());
    if (size > outCap || !req.SerializeToArray(out, static_cast<int>(outCap))) {
        AC_LOG_WARN(kModule, "PICS request too large to rewrite (%u bytes).", size);
        diag::Record("pics_rewrite_failed", std::to_string(size));
        return kNoChange;
    }
    AC_LOG_INFO(kModule, "Injected %d access token(s) into PICS request.", injected);
    return static_cast<std::int32_t>(size);
}

}  // namespace ac::hooks::AccessToken
