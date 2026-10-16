#include "pch.h"
#include "hooks/wire/WireDiag.h"

#include "core/Logger.h"

namespace ac::hooks {
namespace {

constexpr const char* kModule = "Wire";

} // namespace

const char* EresultName(std::int32_t eresult) {
    switch (eresult) {
    case -1:
        return "missing_or_unparsed";
    case 1:
        return "OK";
    case 3:
        return "NoConnection";
    case 15:
        return "AccessDenied";
    case 16:
        return "Timeout";
    case 20:
        return "ServiceUnavailable";
    case 84:
        return "RateLimitExceeded";
    default:
        return "other";
    }
}

const char* EresultClass(std::int32_t eresult) {
    switch (eresult) {
    case 3:
    case 16:
    case 20:
    case 84:
        return "transport_or_provider_candidate";
    case 15:
        return "auth_or_policy_candidate";
    default:
        return "not_classified_as_network";
    }
}

void TraceFrame(const char* dir, const WireFrame& f) {
    AC_LOG_TRACE(kModule, "%s eMsg=%u hLen=%u bLen=%u", dir, f.eMsg, f.headerLen, f.bodyLen);
}

} // namespace ac::hooks
