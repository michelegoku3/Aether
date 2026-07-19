#pragma once

#include <string>
#include <string_view>
#include <vector>

// ---------------------------------------------------------------------------
// Allowlisted HTTP GET for the lcHttpGet Lua binding.
//
// A hostile .lua dropped into the plugin folder could pair this with the
// addappid surface to exfiltrate data, so the host is gated: only the built-in
// baseline hosts plus any in [lua] http_allowlist are reachable. A blocked host
// returns status 403 with an empty body — indistinguishable from a real server
// refusal, so a script cannot probe the gate.
//
// Hard caps: GET only, body <= 8 MiB, total budget 12 s.
// ---------------------------------------------------------------------------
namespace ac::http {

struct Response {
    bool networkError = true;
    int status = 0;       // HTTP status, or 403 when blocked by the gate
    std::string body;     // empty on error / block
};

// Performs a gated GET (host allowlist enforced). For the lcHttpGet binding.
// Never throws.
Response Get(std::string_view url);

// Performs a gated POST (same allowlist as Get). Useful for Lua-driven runtime
// features such as on-demand ETicket minting. Never throws.
Response Post(std::string_view url, std::string_view body,
              const std::vector<std::string>& extraHeaders = {});

// Performs a GET WITHOUT the host allowlist, for internal callers that build the
// URL themselves from trusted config (e.g. the manifest bridge). timeoutSec <= 0
// falls back to the default budget. Optional userAgent overrides the default.
// Never throws.
Response GetUnchecked(std::string_view url, int timeoutSec,
                      std::wstring_view userAgent = L"AetherCore/1.0");

}  // namespace ac::http
