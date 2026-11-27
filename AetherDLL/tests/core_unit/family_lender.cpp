#include "cases.h"

#include <cstdint>
#include <iomanip>
#include <optional>
#include <sstream>
#include <string>
#include <unordered_map>
#include <vector>

#include "family/FamilyLenderPolicy.h"

namespace {

using ac::family::policy::ExtractTicketPrimaryPackageId;
using ac::family::policy::LenderSource;
using ac::family::policy::ResolveLenders;
using ac::steam::AppId;

void AppendLe32(std::string& hex, std::uint32_t v) {
    static constexpr char kHex[] = "0123456789abcdef";
    for (int i = 0; i < 4; ++i) {
        const auto byte = static_cast<std::uint8_t>((v >> (i * 8)) & 0xFFu);
        hex.push_back(kHex[(byte >> 4) & 0xFu]);
        hex.push_back(kHex[byte & 0xFu]);
    }
}

// Builds a syntactically realistic hex-encoded AppOwnershipTicket with the
// exact header offsets Steam stores in localconfig.vdf `apptickets`.
std::string MakeTicketHex(std::uint32_t accountId, AppId appId, std::uint32_t externalIpBe,
                          const std::vector<std::uint32_t>& packages, std::uint32_t signatureOffsetOverride = 0) {
    const std::uint32_t bodyBytes = static_cast<std::uint32_t>(44 + packages.size() * 4 + 4); // +4 trailing dlcCount=0
    const std::uint32_t sigOffset = signatureOffsetOverride != 0 ? signatureOffsetOverride : bodyBytes;
    std::string hex;
    hex.reserve((bodyBytes + 128) * 2);
    AppendLe32(hex, sigOffset);                                   // 0..3: signatureOffset
    AppendLe32(hex, 4);                                           // 4..7: version
    AppendLe32(hex, accountId);                                   // 8..11: steamID low
    AppendLe32(hex, 0x01100001u);                                 // 12..15: steamID high ("01001001")
    AppendLe32(hex, appId);                                       // 16..19: appID
    AppendLe32(hex, externalIpBe);                                // 20..23: ownershipTicketExternalIP
    AppendLe32(hex, 0x0100A8C0u);                                 // 24..27: internal IP
    AppendLe32(hex, 0);                                           // 28..31: flags
    AppendLe32(hex, 1700000000u);                                 // 32..35: generated
    AppendLe32(hex, 1700086400u);                                 // 36..39: expires
    AppendLe32(hex, static_cast<std::uint32_t>(packages.size())); // 40..43: licenseCount
    for (const std::uint32_t pkg : packages)
        AppendLe32(hex, pkg); // 44..: licenses[]
    AppendLe32(hex, 0);       // trailing dlcCount=0
    hex.append(128 * 2, 'a'); // RSA signature bytes
    return hex;
}

void SharedExternalIpDoesNotPropagateLenderAcrossDifferentPackages() {
    // Reproduces the 9dz3pxp Desktop Mate bug: both apps share the user's WAN IP
    // at byte 20, while byte 44 holds completely distinct package IDs.
    constexpr std::uint32_t kSharedExternalIp = 0x5D90A8C0u;
    constexpr AppId kLenderApp = 2358720;
    constexpr std::uint32_t kLenderPkg = 900111;
    constexpr AppId kFreeApp = 3301060;
    constexpr std::uint32_t kFreePkg = 1169138;
    constexpr std::uint32_t kBrotherId = 111222333;

    const auto lenderPkg =
        ExtractTicketPrimaryPackageId(kLenderApp, MakeTicketHex(999u, kLenderApp, kSharedExternalIp, {kLenderPkg}));
    const auto freePkg =
        ExtractTicketPrimaryPackageId(kFreeApp, MakeTicketHex(999u, kFreeApp, kSharedExternalIp, {kFreePkg}));
    CHECK(lenderPkg.has_value());
    CHECK(freePkg.has_value());
    CHECK_EQ(*lenderPkg, kLenderPkg);
    CHECK_EQ(*freePkg, kFreePkg);

    const std::unordered_map<AppId, std::uint32_t> appPackages{{kLenderApp, *lenderPkg}, {kFreeApp, *freePkg}};
    const std::unordered_map<AppId, std::uint32_t> directOwners{{kLenderApp, kBrotherId}};
    const std::unordered_map<std::uint32_t, std::uint32_t> packageOwners{{*lenderPkg, kBrotherId}};

    const auto resolved = ResolveLenders(appPackages, directOwners, packageOwners);
    CHECK(resolved.find(kLenderApp) != resolved.end());
    CHECK_EQ(resolved.at(kLenderApp).lenderAccountId, kBrotherId);
    CHECK(resolved.find(kFreeApp) == resolved.end());
}

void SharedPackagePropagatesLenderToSiblingApp() {
    constexpr AppId kSiblingWithManifestOwner = 1001;
    constexpr AppId kSiblingWithoutManifestOwner = 1002;
    constexpr std::uint32_t kSharedFamilyPkg = 777888;
    constexpr std::uint32_t kBrotherId = 555666;

    // Even if the two tickets were issued on different external IPs, the same
    // single-license package ID propagates the lender reliably.
    const auto pkgA = ExtractTicketPrimaryPackageId(
        kSiblingWithManifestOwner, MakeTicketHex(999u, kSiblingWithManifestOwner, 0x01020304u, {kSharedFamilyPkg}));
    const auto pkgB =
        ExtractTicketPrimaryPackageId(kSiblingWithoutManifestOwner, MakeTicketHex(999u, kSiblingWithoutManifestOwner,
                                                                                  0x05060708u, {kSharedFamilyPkg}));
    CHECK(pkgA.has_value());
    CHECK(pkgB.has_value());
    CHECK_EQ(*pkgA, kSharedFamilyPkg);
    CHECK_EQ(*pkgB, kSharedFamilyPkg);

    const auto resolved = ResolveLenders({{kSiblingWithManifestOwner, *pkgA}, {kSiblingWithoutManifestOwner, *pkgB}},
                                         {{kSiblingWithManifestOwner, kBrotherId}}, {{*pkgA, kBrotherId}});
    CHECK(resolved.find(kSiblingWithoutManifestOwner) != resolved.end());
    CHECK_EQ(resolved.at(kSiblingWithoutManifestOwner).lenderAccountId, kBrotherId);
    CHECK_EQ(resolved.at(kSiblingWithoutManifestOwner).packageId, kSharedFamilyPkg);
    CHECK(resolved.at(kSiblingWithoutManifestOwner).source == LenderSource::SharedPackage);
}

void InvalidAndMultiLicenseTicketsAreIgnored() {
    constexpr AppId kApp = 3301060;
    // Multi-license ticket must not be attributed as a single unambiguous package.
    CHECK(!ExtractTicketPrimaryPackageId(kApp, MakeTicketHex(999u, kApp, 0x11223344u, {100u, 200u})).has_value());
    // Zero package ID is invalid.
    CHECK(!ExtractTicketPrimaryPackageId(kApp, MakeTicketHex(999u, kApp, 0x11223344u, {0u})).has_value());
    // Empty license list is invalid.
    CHECK(!ExtractTicketPrimaryPackageId(kApp, MakeTicketHex(999u, kApp, 0x11223344u, {})).has_value());
    // Mismatched header appID is rejected.
    CHECK(!ExtractTicketPrimaryPackageId(kApp, MakeTicketHex(999u, kApp + 1, 0x11223344u, {1169138u})).has_value());
    // Corrupt signature offset (too small / beyond ticket length) is rejected.
    CHECK(!ExtractTicketPrimaryPackageId(kApp, MakeTicketHex(999u, kApp, 0x11223344u, {1169138u}, 24u)).has_value());
    CHECK(!ExtractTicketPrimaryPackageId(kApp, MakeTicketHex(999u, kApp, 0x11223344u, {1169138u}, 9999u)).has_value());
    // Truncated / non-hex / odd length rejected.
    const auto valid = MakeTicketHex(999u, kApp, 0x11223344u, {1169138u});
    CHECK(!ExtractTicketPrimaryPackageId(kApp, valid.substr(0, 60)).has_value());
    CHECK(!ExtractTicketPrimaryPackageId(kApp, valid.substr(0, valid.size() - 1)).has_value());
    std::string badHex = valid;
    badHex[90] = 'z';
    CHECK(!ExtractTicketPrimaryPackageId(kApp, badHex).has_value());
}

} // namespace

void FamilyLenderTest() {
    SharedExternalIpDoesNotPropagateLenderAcrossDifferentPackages();
    SharedPackagePropagatesLenderToSiblingApp();
    InvalidAndMultiLicenseTicketsAreIgnored();
}
