#include "pch.h"
#include "utils/IpcSpecParse.h"

#include <toml++/toml.hpp>

#include <cctype>
#include <cstdlib>
#include <limits>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>

#include "core/Logger.h"

namespace ac::ipcspec {
namespace {

constexpr const char* kModule = "IpcSpecParse";

// Builds "IClientUser::GetSteamID" from the TOML table path.
std::string QualifiedName(std::string_view iface, std::string_view method) {
    std::string out;
    out.reserve(iface.size() + 2 + method.size());
    out.append(iface);
    out.append("::");
    out.append(method);
    return out;
}

bool ParseHex32(std::string_view text, std::uint32_t& out) {
    if (text.empty()) return false;
    std::string value(text);
    if (value.size() >= 2 && value[0] == '0' &&
        (value[1] == 'x' || value[1] == 'X')) {
        value.erase(0, 2);
    }
    if (value.empty() || value.size() > 8) return false;
    for (unsigned char c : value) {
        if (!std::isxdigit(c)) return false;
    }

    char* end = nullptr;
    const unsigned long parsed = std::strtoul(value.c_str(), &end, 16);
    if (!end || *end != '\0' || parsed == 0 ||
        parsed > std::numeric_limits<std::uint32_t>::max()) {
        return false;
    }
    out = static_cast<std::uint32_t>(parsed);
    return true;
}

// Parses an optional hex fencepost field (e.g. "0x1C"). Returns false when the
// field is present but malformed; leaves *out unchanged on absence.
bool ParseOptionalHex32(std::string_view text, std::uint32_t& out) {
    if (text.empty()) return false;
    return ParseHex32(text, out);
}

}  // namespace

bool ParseSpecToml(const std::string& body,
                   std::unordered_map<std::string, std::uint8_t>& outInterfaceIds,
                   std::unordered_map<std::string, MethodSpec>& outMethods) {
    std::unordered_map<std::string, std::uint8_t> interfaceIds;
    std::unordered_map<std::string, MethodSpec> methods;

    try {
        auto tbl = toml::parse(body);
        for (const auto& [ifaceKey, ifaceNode] : tbl) {
            const std::string ifaceName(ifaceKey.str());
            auto* ifaceTbl = ifaceNode.as_table();
            if (!ifaceTbl || ifaceName.empty()) continue;

            if (auto id = (*ifaceTbl)["interface_id"].value<std::int64_t>()) {
                if (*id <= 0 || *id > 255) {
                    AC_LOG_WARN(kModule, "Invalid interface_id for %s.", ifaceName.c_str());
                    continue;
                }
                interfaceIds.emplace(ifaceName, static_cast<std::uint8_t>(*id));
            }

            for (const auto& [methodKey, methodNode] : *ifaceTbl) {
                const std::string methodName(methodKey.str());
                auto* methodTbl = methodNode.as_table();
                if (!methodTbl || methodName.empty()) continue;

                auto hashStr = (*methodTbl)["funcHash"].value<std::string>();
                if (!hashStr) continue;

                MethodSpec spec{};
                if (!ParseHex32(*hashStr, spec.hash)) {
                    AC_LOG_WARN(kModule, "Invalid funcHash for %s::%s.",
                                ifaceName.c_str(), methodName.c_str());
                    continue;
                }

                // Optional metadata: parse leniently, never fail the method.
                if (auto fencepostStr = (*methodTbl)["fencepost"].value<std::string>()) {
                    if (!ParseOptionalHex32(*fencepostStr, spec.fencepost)) {
                        AC_LOG_WARN(kModule, "Invalid fencepost for %s::%s; ignoring.",
                                    ifaceName.c_str(), methodName.c_str());
                    }
                }
                if (auto argc = (*methodTbl)["argc"].value<std::int64_t>()) {
                    if (*argc >= 0 && *argc <= std::numeric_limits<std::uint32_t>::max()) {
                        spec.argc = static_cast<std::uint32_t>(*argc);
                    } else {
                        AC_LOG_WARN(kModule, "Invalid argc for %s::%s; ignoring.",
                                    ifaceName.c_str(), methodName.c_str());
                    }
                }

                methods.emplace(QualifiedName(ifaceName, methodName), spec);
            }
        }
    } catch (const toml::parse_error& e) {
        AC_LOG_WARN(kModule, "TOML parse error: %s", e.what());
        return false;
    }

    if (methods.empty()) return false;
    outInterfaceIds = std::move(interfaceIds);
    outMethods = std::move(methods);
    return true;
}

}  // namespace ac::ipcspec
