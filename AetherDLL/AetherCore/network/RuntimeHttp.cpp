#include "pch.h"
#include "network/RuntimeHttp.h"

#include <winhttp.h>

#include <algorithm>
#include <array>
#include <cctype>
#include <string>
#include <vector>

#include "core/AetherCoreState.h"
#include "core/Logger.h"

#pragma comment(lib, "winhttp.lib")

namespace ac::http {
namespace {

constexpr const char* kModule = "Http";
constexpr std::size_t kMaxBodyBytes = 8u * 1024u * 1024u;  // 8 MiB
constexpr DWORD kTimeoutMs = 12000;

// Baseline hosts always reachable (manifest sources used by manifest scripts).
constexpr std::array<std::string_view, 3> kBaselineHosts = {
    "raw.githubusercontent.com",
    "cdn.jsdelivr.net",
    "manifesthub1.filegear-sg.me",
};

struct Handle {
    HINTERNET h = nullptr;
    explicit Handle(HINTERNET handle = nullptr) : h(handle) {}
    ~Handle() { if (h) WinHttpCloseHandle(h); }
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    explicit operator bool() const { return h != nullptr; }
};

bool EqualsIgnoreCase(std::string_view a, std::string_view b) {
    if (a.size() != b.size()) return false;
    for (std::size_t i = 0; i < a.size(); ++i) {
        if (std::tolower(static_cast<unsigned char>(a[i])) !=
            std::tolower(static_cast<unsigned char>(b[i]))) {
            return false;
        }
    }
    return true;
}

// Extracts the host portion of an http(s) URL (no scheme, port, or path).
std::string_view ExtractHost(std::string_view url) {
    for (std::string_view scheme : {std::string_view("https://"), std::string_view("http://")}) {
        if (url.size() >= scheme.size() && EqualsIgnoreCase(url.substr(0, scheme.size()), scheme)) {
            url.remove_prefix(scheme.size());
            std::size_t end = url.size();
            for (std::size_t i = 0; i < url.size(); ++i) {
                char c = url[i];
                if (c == '/' || c == '?' || c == '#' || c == ':') { end = i; break; }
            }
            return url.substr(0, end);
        }
    }
    return {};
}

bool IsHostAllowed(std::string_view host) {
    if (host.empty()) return false;
    for (auto h : kBaselineHosts) {
        if (EqualsIgnoreCase(host, h)) return true;
    }
    for (const auto& h : g_state.settings.httpAllowlistExtra) {
        if (EqualsIgnoreCase(host, h)) return true;
    }
    return false;
}

// Converts a UTF-8 narrow string to a wide string using the Windows API.
// The previous byte-by-byte copy (std::wstring(s.begin(), s.end())) would
// mangle any non-ASCII character by treating each UTF-8 continuation byte
// as a separate wchar_t, producing bogus host/path strings for WinHTTP.
std::wstring Widen(std::string_view s) {
    if (s.empty()) return {};
    int needed = MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), nullptr, 0);
    if (needed <= 0) return {};
    std::wstring out(static_cast<std::size_t>(needed), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), &out[0], needed);
    return out;
}

std::wstring JoinHeaders(const std::vector<std::string>& headers) {
    if (headers.empty()) return {};
    std::string joined;
    for (const auto& h : headers) {
        if (h.empty()) continue;
        joined += h;
        joined += "\r\n";
    }
    return Widen(joined);
}

// Performs the actual request with the given total timeout. No allowlist check
// — callers that need the gate apply it first.
Response Request(std::wstring_view method, std::string_view url, DWORD timeoutMs,
                 std::wstring_view userAgent, std::string_view requestBody,
                 const std::vector<std::string>& headers) {
    Response out;

    std::string_view host = ExtractHost(url);
    if (host.empty()) return out;

    std::string_view rest = url;
    bool https = true;
    if (EqualsIgnoreCase(rest.substr(0, 8), "https://")) { rest.remove_prefix(8); https = true; }
    else if (EqualsIgnoreCase(rest.substr(0, 7), "http://")) { rest.remove_prefix(7); https = false; }
    std::size_t slash = rest.find('/');
    std::string_view path = (slash == std::string_view::npos) ? "/" : rest.substr(slash);

    std::wstring wHost = Widen(host);
    std::wstring wPath = Widen(path);
    INTERNET_PORT port = https ? INTERNET_DEFAULT_HTTPS_PORT : INTERNET_DEFAULT_HTTP_PORT;

    std::wstring ua = userAgent.empty() ? std::wstring(L"AetherCore/1.0")
                                        : std::wstring(userAgent);
    std::wstring wHeaders = JoinHeaders(headers);
    std::wstring wMethod = method.empty() ? std::wstring(L"GET") : std::wstring(method);
    Handle session(WinHttpOpen(ua.c_str(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                               WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0));
    if (!session) return out;
    WinHttpSetTimeouts(session.h, timeoutMs, timeoutMs, timeoutMs, timeoutMs);

    Handle connect(WinHttpConnect(session.h, wHost.c_str(), port, 0));
    if (!connect) return out;

    Handle request(WinHttpOpenRequest(connect.h, wMethod.c_str(), wPath.c_str(), nullptr,
                                      WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES,
                                      https ? WINHTTP_FLAG_SECURE : 0));
    if (!request) return out;

    const wchar_t* hdrPtr = wHeaders.empty() ? WINHTTP_NO_ADDITIONAL_HEADERS : wHeaders.c_str();
    DWORD hdrLen = wHeaders.empty() ? 0 : static_cast<DWORD>(wHeaders.size());
    LPVOID bodyPtr = requestBody.empty() ? WINHTTP_NO_REQUEST_DATA : const_cast<char*>(requestBody.data());
    DWORD bodyLen = static_cast<DWORD>(requestBody.size());

    if (!WinHttpSendRequest(request.h, hdrPtr, hdrLen, bodyPtr, bodyLen, bodyLen, 0) ||
        !WinHttpReceiveResponse(request.h, nullptr)) {
        return out;
    }

    DWORD status = 0, statusSize = sizeof(status);
    WinHttpQueryHeaders(request.h, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                        WINHTTP_HEADER_NAME_BY_INDEX, &status, &statusSize, WINHTTP_NO_HEADER_INDEX);

    std::string body;
    while (true) {
        DWORD avail = 0;
        if (!WinHttpQueryDataAvailable(request.h, &avail) || avail == 0) break;
        if (body.size() + avail > kMaxBodyBytes) {
            AC_LOG_WARN(kModule, "Body exceeds 8 MiB cap; aborting.");
            return out;  // networkError stays true: caller sees a failed fetch
        }
        std::vector<char> chunk(avail);
        DWORD read = 0;
        if (!WinHttpReadData(request.h, chunk.data(), avail, &read) || read == 0) break;
        body.append(chunk.data(), read);
    }

    out.networkError = false;
    out.status = static_cast<int>(status);
    out.body = std::move(body);
    return out;
}

}  // namespace

Response Get(std::string_view url) {
    std::string_view host = ExtractHost(url);
    if (!IsHostAllowed(host)) {
        // Blocked: mimic a server refusal so scripts can't probe the gate.
        AC_LOG_WARN(kModule, "Blocked host '%.*s'.", static_cast<int>(host.size()), host.data());
        Response out;
        out.networkError = false;
        out.status = 403;
        return out;
    }
    return Request(L"GET", url, kTimeoutMs, L"AetherCore/1.0", {}, {});
}

Response Post(std::string_view url, std::string_view body,
              const std::vector<std::string>& extraHeaders) {
    std::string_view host = ExtractHost(url);
    if (!IsHostAllowed(host)) {
        AC_LOG_WARN(kModule, "Blocked host '%.*s'.", static_cast<int>(host.size()), host.data());
        Response out;
        out.networkError = false;
        out.status = 403;
        return out;
    }
    return Request(L"POST", url, kTimeoutMs, L"AetherCore/1.0", body, extraHeaders);
}

Response GetUnchecked(std::string_view url, int timeoutSec, std::wstring_view userAgent) {
    DWORD ms = timeoutSec > 0 ? static_cast<DWORD>(timeoutSec) * 1000u : kTimeoutMs;
    return Request(L"GET", url, ms, userAgent, {}, {});
}

}  // namespace ac::http
