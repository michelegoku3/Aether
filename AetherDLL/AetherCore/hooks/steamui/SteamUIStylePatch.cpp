#include "pch.h"
#include "hooks/steamui/SteamUIStylePatch.h"

#include <fstream>
#include <iterator>
#include <string>
#include <string_view>

#include "core/Logger.h"
#include "core/SteamUiCssText.h"
#include "utils/WinFile.h"

namespace ac::steamui {
namespace {

constexpr const char* kModule = "SteamUI.Css";

constexpr std::string_view kMappingKey = "OldSharingWarning:\"";

bool ReadTextFile(const std::string& path, std::string& out) {
    std::ifstream input(winfile::ToPath(path), std::ios::binary);
    if (!input.is_open())
        return false;
    out.assign(std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>());
    return input.good() || input.eof();
}

bool WriteTextAtomically(const std::string& path, const std::string& content) {
    const std::string temp = path + ".aether-ui.tmp";
    {
        std::ofstream output(winfile::ToPath(temp), std::ios::binary | std::ios::trunc);
        if (!output.is_open())
            return false;
        output.write(content.data(), static_cast<std::streamsize>(content.size()));
        output.flush();
        if (!output.good()) {
            output.close();
            winfile::Delete(temp);
            return false;
        }
    }

    const std::wstring wide = winfile::WidenPath(path);
    const DWORD attributes = GetFileAttributesW(wide.c_str());
    const bool readOnly = attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_READONLY) != 0;
    if (readOnly)
        SetFileAttributesW(wide.c_str(), attributes & ~FILE_ATTRIBUTE_READONLY);

    const bool replaced = winfile::MoveReplace(temp, path);

    if (readOnly && attributes != INVALID_FILE_ATTRIBUTES)
        SetFileAttributesW(wide.c_str(), attributes);
    if (!replaced)
        winfile::Delete(temp);
    return replaced;
}

// CSS-module hashes are `[A-Za-z0-9_-]+`; reject anything else so a malformed
// mapping can never inject arbitrary text into the stylesheet.
bool IsValidCssClass(std::string_view value) {
    if (value.empty() || value.size() > 64)
        return false;
    for (const char c : value) {
        const bool ok =
            (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '-';
        if (!ok)
            return false;
    }
    return true;
}

// Returns a copy: a view into the (5-16 MB) script would dangle as soon as the
// caller's buffer goes out of scope.
std::string ResolveWarningClass(const std::string& spJs) {
    const std::size_t key = spJs.find(kMappingKey);
    if (key == std::string::npos)
        return {};
    const std::size_t start = key + kMappingKey.size();
    const std::size_t end = spJs.find('"', start);
    if (end == std::string::npos)
        return {};
    return spJs.substr(start, end - start);
}

} // namespace

void ApplyLegacySharingWarningPolicy(const std::string& steamInstallPath, bool hide) {
    if (steamInstallPath.empty()) {
        AC_LOG_DEBUG(kModule, "Steam path not resolved; stylesheet left untouched.");
        return;
    }

    const std::string uiRoot = steamInstallPath + "\\steamui";
    const std::string spCssPath = uiRoot + "\\css\\sp.css";

    std::string css;
    if (!ReadTextFile(spCssPath, css)) {
        AC_LOG_DEBUG(kModule, "Cannot read %s; stylesheet left untouched.", spCssPath.c_str());
        return;
    }

    // Dropping the legacy marker block restores the stock byte count. It is
    // independent of the warning patch and must be persisted even when the
    // rule cannot be patched: that extra block is exactly the size mismatch
    // that makes Steam reinstall the client.
    bool migrated = css::RemoveLegacyBlock(css);
    const auto writeMigration = [&] {
        if (!migrated)
            return;
        if (!WriteTextAtomically(spCssPath, css)) {
            AC_LOG_WARN(kModule, "Cannot write %s after removing the legacy marker block; stylesheet left unchanged.",
                        spCssPath.c_str());
            return;
        }
        migrated = false;
        AC_LOG_INFO(kModule, "Legacy sharing warning block removed (%zu bytes, stock size restored).", css.size());
    };

    std::string cssClass;
    {
        // Both directions need the class only to locate the rule; it is
        // content-hashed per Steam build, so it always comes from sp.js.
        const std::string spJsPath = uiRoot + "\\sp.js";
        std::string spJs;
        if (!ReadTextFile(spJsPath, spJs)) {
            AC_LOG_DEBUG(kModule, "Cannot read %s; stylesheet left untouched.", spJsPath.c_str());
            writeMigration();
            return;
        }
        cssClass = ResolveWarningClass(spJs);
        if (!IsValidCssClass(cssClass)) {
            if (hide)
                AC_LOG_WARN(kModule, "OldSharingWarning mapping not found in sp.js; Steam UI left untouched.");
            writeMigration();
            return; // nothing was hidden through this code path: nothing to restore
        }
    }

    const css::WarningPatch result = css::ApplyWarningPatch(css, cssClass, hide);
    if (result == css::WarningPatch::Unchanged) {
        if (!migrated) {
            AC_LOG_DEBUG(kModule, "Legacy sharing warning already in the requested state.");
            return;
        }
        writeMigration();
        return;
    }
    if (result == css::WarningPatch::NotApplicable) {
        AC_LOG_WARN(kModule, "OldSharingWarning rule '%s' not found in sp.css; Steam UI left untouched.",
                    cssClass.c_str());
        writeMigration();
        return;
    }

    if (!WriteTextAtomically(spCssPath, css)) {
        AC_LOG_WARN(kModule, "Cannot write %s; stylesheet left unchanged.", spCssPath.c_str());
        return;
    }
    migrated = false;
    AC_LOG_INFO(kModule, "Legacy sharing warning %s (class %s, %zu bytes, size-preserving).",
                hide ? "hidden" : "restored", cssClass.c_str(), css.size());
}

} // namespace ac::steamui
