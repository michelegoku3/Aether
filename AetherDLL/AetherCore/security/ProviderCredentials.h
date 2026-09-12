#pragma once

#include <optional>
#include <string>

namespace ac::security {

// Reads the provider credential file written by AetherDesk and decrypts it
// with the same Windows DPAPI CurrentUser scope. No credential is persisted by
// AetherCore and the returned value must never be logged.
std::optional<std::string> ReadHubcapApiKey();

}  // namespace ac::security
