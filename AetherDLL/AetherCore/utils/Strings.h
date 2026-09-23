#pragma once
#include <limits>
#include <string>
#include <string_view>
#ifdef _WIN32
#include <windows.h>
#endif
namespace ac::strings {
constexpr unsigned char LowerAscii(unsigned char c) {
    return c >= 'A' && c <= 'Z' ? static_cast<unsigned char>(c + ('a' - 'A')) : c;
}
inline bool EqualsIgnoreCase(std::string_view a, std::string_view b) {
    if (a.size() != b.size()) return false;
    for (std::size_t i = 0; i < a.size(); ++i)
        if (LowerAscii(static_cast<unsigned char>(a[i])) !=
            LowerAscii(static_cast<unsigned char>(b[i]))) return false;
    return true;
}
inline std::string Trim(std::string_view value) {
    constexpr auto whitespace = " \t\r\n\f\v";
    const auto begin = value.find_first_not_of(whitespace);
    if (begin == std::string_view::npos) return {};
    return std::string(value.substr(begin, value.find_last_not_of(whitespace) - begin + 1));
}
// HTTP(S) DNS/IPv4 authority only, not a general URL parser. Reject userinfo
// anywhere in the authority (also after a port). View borrows the input URL.
inline std::string_view ExtractHost(std::string_view url) {
    if (EqualsIgnoreCase(url.substr(0, 8), "https://")) url.remove_prefix(8);
    else if (EqualsIgnoreCase(url.substr(0, 7), "http://")) url.remove_prefix(7);
    else return {};
    const auto authority = url.substr(0, url.find_first_of("/?#"));
    if (authority.empty() || authority.find_first_of("@\\[]") != std::string_view::npos) return {};
    for (unsigned char c : authority) if (c <= 0x20 || c == 0x7f) return {};
    const auto colon = authority.find(':');
    if (colon != std::string_view::npos) {
        const auto port = authority.substr(colon + 1);
        if (port.empty()) return {};
        unsigned value = 0;
        for (char c : port) {
            if (c < '0' || c > '9') return {};
            value = value * 10 + static_cast<unsigned>(c - '0');
            if (value > 65535) return {};
        }
        if (value == 0) return {};
    }
    return authority.substr(0, colon);
}
#ifdef _WIN32
// Strict UTF-8, not byte-by-byte widening. Invalid input is rejected.
inline std::wstring Widen(std::string_view text) {
    if (text.empty() || text.size() > static_cast<std::size_t>((std::numeric_limits<int>::max)())) return {};
    const int count = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text.data(),
                                         static_cast<int>(text.size()), nullptr, 0);
    if (count <= 0) return {};
    std::wstring out(static_cast<std::size_t>(count), L'\0');
    if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text.data(),
                           static_cast<int>(text.size()), out.data(), count) != count) return {};
    return out;
}
#endif
}  // namespace ac::strings
