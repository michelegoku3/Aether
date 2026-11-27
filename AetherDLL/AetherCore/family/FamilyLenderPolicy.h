#pragma once

#include <charconv>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string_view>
#include <unordered_map>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// App-ownership ticket package extraction and Steam Family lender resolution.
//
// Steam stores decrypted AppOwnershipTickets as lowercase hex strings in
// localconfig.vdf (`apptickets`). Binary layout (all integers little-endian):
//   0..3   signatureOffset (size of ticket body preceding RSA signature)
//   4..7   ticket version (usually 4)
//   8..15  steamID (bytes 12..15 = 01 00 10 01 -> hex "01001001")
//   16..19 appID
//   20..23 ownershipTicketExternalIP
//   24..39 internal IP, ownership flags, generated/expires timestamps
//   40..43 licenseCount
//   44..   licenses[licenseCount] (uint32 package IDs / SubIDs)
//
// Header-only and pure: tested directly by core_unit_tests.
// ---------------------------------------------------------------------------
namespace ac::family::policy {

inline constexpr std::size_t kMinAppTicketHexChars = 96; // 44B header + 4B first license
inline constexpr std::size_t kTicketSteamIdUpperHexChar = 24;
inline constexpr std::size_t kTicketSignatureOffsetByte = 0;
inline constexpr std::size_t kTicketAppIdByte = 16;
inline constexpr std::size_t kTicketLicenseCountByte = 40;
inline constexpr std::size_t kTicketFirstLicenseByte = 44;

enum class LenderSource { None, DirectManifest, SharedPackage };

struct ResolvedLender {
    std::uint32_t lenderAccountId = 0;
    std::uint32_t packageId = 0;
    LenderSource source = LenderSource::None;
};

inline std::optional<std::uint32_t> ParseBeHex32(std::string_view hex8) {
    if (hex8.size() != 8)
        return std::nullopt;
    std::uint32_t value = 0;
    const auto parsed = std::from_chars(hex8.data(), hex8.data() + hex8.size(), value, 16);
    if (parsed.ec != std::errc{} || parsed.ptr != hex8.data() + hex8.size())
        return std::nullopt;
    return value;
}

inline std::optional<std::uint32_t> ParseLeHex32AtByte(std::string_view hexTicket, std::size_t byteOffset) {
    const std::size_t hexOffset = byteOffset * 2;
    if (hexOffset + 8 > hexTicket.size())
        return std::nullopt;
    const auto beValue = ParseBeHex32(hexTicket.substr(hexOffset, 8));
    if (!beValue)
        return std::nullopt;
    const std::uint32_t v = *beValue;
    return ((v & 0x000000FFu) << 24) | ((v & 0x0000FF00u) << 8) | ((v & 0x00FF0000u) >> 8) | ((v & 0xFF000000u) >> 24);
}

inline std::optional<std::uint32_t> ExtractTicketPrimaryPackageId(steam::AppId expectedAppId,
                                                                  std::string_view hexTicket) {
    if (expectedAppId == 0 || hexTicket.size() < kMinAppTicketHexChars || hexTicket.size() % 2 != 0 ||
        hexTicket.compare(kTicketSteamIdUpperHexChar, 8, "01001001") != 0)
        return std::nullopt;

    const std::size_t ticketBytes = hexTicket.size() / 2;
    const auto signatureOffset = ParseLeHex32AtByte(hexTicket, kTicketSignatureOffsetByte);
    const auto ticketAppId = ParseLeHex32AtByte(hexTicket, kTicketAppIdByte);
    const auto licenseCount = ParseLeHex32AtByte(hexTicket, kTicketLicenseCountByte);
    const auto packageId = ParseLeHex32AtByte(hexTicket, kTicketFirstLicenseByte);
    if (!signatureOffset || !ticketAppId || !licenseCount || !packageId)
        return std::nullopt;

    // Require an unambiguous single-license ticket belonging to expectedAppId
    // whose unsigned body fits within signatureOffset and ticketBytes; reading
    // earlier header bytes (e.g. external IP at offset 20) or guessing across
    // multi-license bundles causes unrelated apps to share a false package key.
    if (*ticketAppId != expectedAppId || *licenseCount != 1 || *packageId == 0 ||
        *signatureOffset < kTicketFirstLicenseByte + 4 || *signatureOffset > ticketBytes)
        return std::nullopt;

    return packageId;
}

inline std::unordered_map<steam::AppId, ResolvedLender>
ResolveLenders(const std::unordered_map<steam::AppId, std::uint32_t>& appPackages,
               const std::unordered_map<steam::AppId, std::uint32_t>& directOwners,
               const std::unordered_map<std::uint32_t, std::uint32_t>& packageOwners) {
    std::unordered_map<steam::AppId, ResolvedLender> resolved;
    for (const auto& [app, package] : appPackages) {
        const auto direct = directOwners.find(app);
        if (direct != directOwners.end()) {
            resolved[app] = {direct->second, package, LenderSource::DirectManifest};
            continue;
        }
        const auto owner = packageOwners.find(package);
        if (owner != packageOwners.end())
            resolved[app] = {owner->second, package, LenderSource::SharedPackage};
    }
    for (const auto& [app, owner] : directOwners) {
        if (resolved.find(app) == resolved.end())
            resolved[app] = {owner, 0, LenderSource::DirectManifest};
    }
    return resolved;
}

} // namespace ac::family::policy
