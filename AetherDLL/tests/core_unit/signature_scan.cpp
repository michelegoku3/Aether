#include "cases.h"

#include "utils/SignatureCodec.h"
#include "utils/SignatureScan.h"

void SignatureScanTest() {
    using ac::pattern::FindUniqueSignatureMatch;
    using ac::pattern::ParseSignature;
    using ac::pattern::ScanResult;
    using ac::pattern::ScanStatus;

    std::vector<std::uint8_t> bytes;
    std::string mask;
    CHECK(ParseSignature("40 53 55 ?? 57 48 83 EC 48", bytes, mask));

    // Unique match with wildcard.
    const std::uint8_t buf1[] = {0xCC, 0xCC, 0x40, 0x53, 0x55, 0x99, 0x57, 0x48, 0x83, 0xEC, 0x48, 0xC3};
    const ScanResult r1 = FindUniqueSignatureMatch(buf1, sizeof(buf1), bytes, mask);
    CHECK(r1.status == ScanStatus::Unique);
    CHECK_EQ(r1.offset, std::size_t{2});

    // No match.
    const std::uint8_t buf2[] = {0x40, 0x53, 0x55, 0x99, 0x57, 0x48, 0x83, 0xEC, 0x49};
    const ScanResult r2 = FindUniqueSignatureMatch(buf2, sizeof(buf2), bytes, mask);
    CHECK(r2.status == ScanStatus::NotFound);

    // Ambiguous (two matches): must refuse rather than guess.
    const std::uint8_t buf3[] = {
        0x40, 0x53, 0x55, 0x56, 0x57, 0x48, 0x83, 0xEC, 0x48, 0xCC,
        0x40, 0x53, 0x55, 0xAA, 0x57, 0x48, 0x83, 0xEC, 0x48,
    };
    const ScanResult r3 = FindUniqueSignatureMatch(buf3, sizeof(buf3), bytes, mask);
    CHECK(r3.status == ScanStatus::Ambiguous);

    // Pattern longer than buffer -> NotFound.
    const std::uint8_t shortBuf[] = {0x40, 0x53};
    const ScanResult r4 = FindUniqueSignatureMatch(shortBuf, sizeof(shortBuf), bytes, mask);
    CHECK(r4.status == ScanStatus::NotFound);

    // Invalid inputs -> Invalid.
    CHECK(FindUniqueSignatureMatch(nullptr, 10, bytes, mask).status == ScanStatus::Invalid);
    CHECK(FindUniqueSignatureMatch(buf1, sizeof(buf1), {}, {}).status == ScanStatus::Invalid);
    CHECK(FindUniqueSignatureMatch(buf1, sizeof(buf1), bytes, "xx").status == ScanStatus::Invalid);
}
