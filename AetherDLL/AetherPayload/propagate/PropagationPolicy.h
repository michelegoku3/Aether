#pragma once
#include <string>
#include <string_view>

namespace ac::payloadprop {
// Conservative scope reduction, NOT an anti-cheat detector or a guarantee of
// compatibility. Unknown identities and system executables are never touched.
inline std::wstring NormalizeImagePath(std::wstring_view path) {
    std::wstring out(path);
    for (auto& c : out) {
        if (c == L'/') c = L'\\';
        else if (c >= L'A' && c <= L'Z') c += L'a' - L'A';
    }
    return out;
}
inline const char* BlockedReason(std::wstring_view image, std::wstring_view windowsDir) {
    if (image.empty() || windowsDir.empty()) return "identity_unavailable";
    const auto path = NormalizeImagePath(image);
    auto root = NormalizeImagePath(windowsDir);
    while (!root.empty() && root.back() == L'\\') root.pop_back();
    if (root.empty()) return "identity_unavailable";
    if (path == root || (path.size() > root.size() &&
        path.compare(0, root.size(), root) == 0 && path[root.size()] == L'\\'))
        return "system_directory";
    const auto slash = path.find_last_of(L'\\');
    if (slash == std::wstring::npos) return "identity_unavailable";
    const auto name = std::wstring_view(path).substr(slash + 1);
    if (name.empty()) return "identity_unavailable";
    for (auto denied : {L"easyanticheat", L"battleye", L"beservice", L"eac_launcher",
                        L"vgc.exe", L"vgtray.exe"}) {
        if (name.find(denied) != std::wstring_view::npos) return "protected_helper";
    }
    for (auto denied : {L"crashpad", L"crashreport", L"crashhandler", L"werfault"}) {
        if (name.find(denied) != std::wstring_view::npos) return "crash_helper";
    }
    for (auto denied : {L"chrome.exe", L"msedge.exe", L"msedgewebview2.exe", L"firefox.exe",
                        L"steamwebhelper.exe", L"cefsubprocess.exe", L"cefsharp.browsersubprocess.exe"}) {
        if (name == denied) return "browser_helper";
    }
    return nullptr;
}
}  // namespace ac::payloadprop
