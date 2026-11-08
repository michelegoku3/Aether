#include "cases.h"

#include "security/HubcapKeyCheck.h"

// The saved-key policy shared with AetherDesk: only a definitive provider
// answer may raise the startup warning. Everything the network can fail with
// stays unknown so an outage is never reported as an expired key.
void HubcapKeyPolicyTest() {
    using ac::hubcapkey::Classify;
    using ac::hubcapkey::Reason;
    using ac::hubcapkey::State;

    // 200 + true or a body without the flag: the key works.
    CHECK(Classify(200, false, R"({"can_make_requests":true,"daily_usage":3})").state == State::Active);
    CHECK(Classify(200, false, R"({"daily_usage":3})").state == State::Active);
    CHECK(Classify(204, false, "").state == State::Active);

    // 200 + false: authenticated, but the account may not generate.
    const auto inactive = Classify(200, false, R"({"can_make_requests":false})");
    CHECK(inactive.state == State::Inactive);
    CHECK(inactive.reason == Reason::AccountInactive);

    // Rejected credentials.
    for (int status : {401, 403}) {
        const auto verdict = Classify(status, false, "{\"detail\":\"expired\"}");
        CHECK(verdict.state == State::Inactive);
        CHECK(verdict.reason == Reason::Unauthorized);
    }

    // Anything else is inconclusive: no warning may be shown.
    for (int status : {408, 425, 429, 500, 502, 503}) {
        const auto verdict = Classify(status, false, "");
        CHECK(verdict.state == State::Unknown);
        CHECK(verdict.reason == Reason::ProviderError);
    }
    const auto offline = Classify(0, true, "");
    CHECK(offline.state == State::Unknown);
    CHECK(offline.reason == Reason::Unreachable);

    // A transport failure wins over any leftover body.
    CHECK(Classify(200, true, R"({"can_make_requests":false})").state == State::Unknown);

    // status.json spellings.
    CHECK(std::string(ac::hubcapkey::StateName(State::Unknown)) == "unknown");
    CHECK(std::string(ac::hubcapkey::StateName(State::Active)) == "active");
    CHECK(std::string(ac::hubcapkey::StateName(State::Inactive)) == "inactive");
    CHECK(std::string(ac::hubcapkey::ReasonName(Reason::None)) == "none");
    CHECK(std::string(ac::hubcapkey::ReasonName(Reason::Unauthorized)) == "unauthorized");
    CHECK(std::string(ac::hubcapkey::ReasonName(Reason::AccountInactive)) == "account_inactive");
    CHECK(std::string(ac::hubcapkey::ReasonName(Reason::Unreachable)) == "unreachable");
    CHECK(std::string(ac::hubcapkey::ReasonName(Reason::ProviderError)) == "provider_error");
}
