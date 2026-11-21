#pragma once

#include <string_view>

#include "utils/JsonStringField.h"

// ---------------------------------------------------------------------------
// Saved Hubcap key health, as AetherDLL can observe it.
//
// The provider exposes no expiry date: the only authoritative signals are
// HTTP 401/403 and `can_make_requests == false` on `/user/stats`. A transport
// failure or any other HTTP status stays `unknown`, so an outage can never be
// reported to the player as an expired key.
//
// Classify is inline and dependency-free (no Win32, no network) so the policy
// is unit-tested directly; the check itself runs on a named worker at boot.
// ---------------------------------------------------------------------------
namespace ac::hubcapkey {

enum class State : int {
    Unknown = 0, // never asked, or the provider could not answer
    Active = 1,
    Inactive = 2, // 401/403, or the account may not make requests
};

enum class Reason {
    None,
    Unauthorized,
    AccountInactive,
    Unreachable,
    ProviderError,
};

struct Verdict {
    State state = State::Unknown;
    Reason reason = Reason::None;
};

// The shared classification, mirrored by AetherDesk's
// `providers/hubcap/key_status.rs`:
//   401/403                              -> inactive/unauthorized
//   2xx + can_make_requests == false     -> inactive/account_inactive
//   2xx + true or field absent           -> active
//   transport failure                    -> unknown/unreachable
//   anything else (408/429/5xx/…)        -> unknown/provider_error
inline Verdict Classify(int status, bool networkError, std::string_view body) {
    if (networkError)
        return Verdict{State::Unknown, Reason::Unreachable};
    if (status == 401 || status == 403)
        return Verdict{State::Inactive, Reason::Unauthorized};
    if (status >= 200 && status < 300) {
        bool canMakeRequests = true;
        if (jsonutil::PullBoolField(body, "can_make_requests", canMakeRequests) && !canMakeRequests)
            return Verdict{State::Inactive, Reason::AccountInactive};
        return Verdict{State::Active, Reason::None};
    }
    return Verdict{State::Unknown, Reason::ProviderError};
}

inline const char* StateName(State state) {
    switch (state) {
    case State::Active:
        return "active";
    case State::Inactive:
        return "inactive";
    default:
        return "unknown";
    }
}

inline const char* ReasonName(Reason reason) {
    switch (reason) {
    case Reason::Unauthorized:
        return "unauthorized";
    case Reason::AccountInactive:
        return "account_inactive";
    case Reason::Unreachable:
        return "unreachable";
    case Reason::ProviderError:
        return "provider_error";
    default:
        return "none";
    }
}

// Starts the one-shot startup check on its own named worker: reads the saved
// key (no key -> nothing to check), asks `/user/stats`, stores the verdict in
// g_state for status.json, and shows ONE warning box when the key is inactive.
// `showNotice=false` keeps the probe and the status but suppresses the box: a
// Steam beta client already raises its own startup notice, and Steam must never
// open two popups at once. Never blocks Steam's startup thread and never logs
// the key. The flag has no default so every call site states the decision.
bool StartCheck(bool showNotice);

} // namespace ac::hubcapkey
