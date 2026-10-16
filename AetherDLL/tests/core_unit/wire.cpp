#include "cases.h"

#include "core/Constants.h"
#include "hooks/ipc/IpcReply.h"

#include "hooks/wire/DonorPool.h"

#include "hooks/wire/GamesPlayedFormat.h"

#include "hooks/wire/SchemaBuckets.h"

#include "hooks/wire/UserStatsSnapshot.h"

#include "network/ManifestIdentity.h"

#include "utils/IpcSpecParse.h"

#include "utils/KeyValues.h"

void SchemaBuckets() {

    auto fixture = [](const std::string& bucket, const std::string& child, bool entry,

                      const std::string& section = "stats") {
        std::vector<std::uint8_t> data;

        auto dict = [&](const std::string& name) {
            data.push_back(0);

            data.insert(data.end(), name.begin(), name.end());

            data.push_back(0);
        };

        dict("123");

        dict(section);

        dict(bucket);

        dict(child);

        if (entry) {

            dict("0");

            data.push_back(8);
        }

        for (int i = 0; i < 5; ++i)

            data.push_back(8);

        return data;
    };

    auto parse = [](const std::vector<std::uint8_t>& data) {
        ac::backup::statscache::detail::SchemaBucketWalker walker;

        CHECK(ac::kv1::WalkBinary(data.data(), data.size(), walker));

        return walker.buckets;
    };

    CHECK(parse(fixture("0", "bits", true)).count(0) == 1);

    CHECK(parse(fixture("200", "bits", true)).count(200) == 1);

    CHECK(parse(fixture("200", "bits", false)).empty());

    CHECK(parse(fixture("200", "display", true)).empty());

    CHECK(parse(fixture("200", "bits", true, "other")).empty());

    for (const auto* bad : {"", "no", "-1", "1x", "4294967296"})

        CHECK(parse(fixture(bad, "bits", true)).empty());

    auto truncated = fixture("200", "bits", true);

    truncated.resize(6); // truncated key; no completed bucket

    ac::backup::statscache::detail::SchemaBucketWalker walker;

    CHECK(!ac::kv1::WalkBinary(truncated.data(), truncated.size(), walker));

    CHECK(walker.buckets.empty());
}

void GamesPlayedFormatTest() {

    namespace fmt = ac::hooks::GamesPlayedFormat;

    // AppIdFromGameId: low-24 mask.

    CHECK(fmt::AppIdFromGameId(0x020000000000ull | 1687950u) == 1687950u);

    CHECK(fmt::AppIdFromGameId(480u) == 480u);

    // ImageStem: path strip, Unreal suffix, trailing "-steam".

    CHECK(fmt::ImageStem("C:\\games\\Bodycam-Win64-Shipping.exe") == "Bodycam");

    CHECK(fmt::ImageStem("ReadyOrNotSteam-Win64-Shipping.exe") == "ReadyOrNot");

    CHECK(fmt::ImageStem("P5R.exe") == "P5R");

    CHECK(fmt::ImageStem("") == "");

    // NameIsUsable: rejects empty and the Spacewar fold.

    CHECK(!fmt::NameIsUsable(""));

    CHECK(!fmt::NameIsUsable("Spacewar"));

    CHECK(!fmt::NameIsUsable("S p a c e w a r"));

    CHECK(fmt::NameIsUsable("Persona 5 Royal"));

    // MakeAppIdBlob: magic + version + LE appid.

    const std::string blob = fmt::MakeAppIdBlob(0x01020304u);

    CHECK(blob.size() == 4 + 1 + 4);

    CHECK(blob.compare(0, 4, ac::constants::kAppIdBlobMagic) == 0);

    CHECK(static_cast<std::uint8_t>(blob[4]) == ac::constants::kAppIdBlobVersion);

    CHECK(static_cast<std::uint8_t>(blob[5]) == 0x04);

    CHECK(static_cast<std::uint8_t>(blob[6]) == 0x03);

    CHECK(static_cast<std::uint8_t>(blob[7]) == 0x02);

    CHECK(static_cast<std::uint8_t>(blob[8]) == 0x01);

    // WithAppIdSuffix (invisible): mark + 6 VS nibbles encoding the appid.

    ac::Settings st;

    st.presenceSuffixInvisible = true;

    const std::string inv = fmt::WithAppIdSuffix("Game", 0xABCDEFu, st);

    CHECK(inv.rfind("Game", 0) == 0);

    CHECK(inv.find(ac::constants::kExtraInfoInvisibleMark) == 4);

    std::uint32_t decoded = 0;

    std::size_t pos = 4 + 3; // skip name + mark

    for (std::size_t i = 0; i < ac::constants::kExtraInfoInvisibleDigits; ++i) {

        CHECK(pos + 3 <= inv.size());

        CHECK(static_cast<std::uint8_t>(inv[pos]) == 0xEE);

        CHECK(static_cast<std::uint8_t>(inv[pos + 1]) == 0xB8);

        const std::uint32_t nib = static_cast<std::uint8_t>(inv[pos + 2]) & 0x0Fu;

        decoded = (decoded << 4) | nib;

        pos += 3;
    }

    CHECK(decoded == 0xABCDEFu);

    st.presenceSuffixInvisible = false;

    CHECK(fmt::WithAppIdSuffix("Game", 480u, st) == std::string("Game") + ac::constants::kExtraInfoAppIdSep + "480");

    // ExtractStringKVs: nested struct + string pairs + end markers.

    const std::uint8_t kv[] = {

        0x00, 'a', 0, // struct "a"

        0x01, 'k', '1', 0, 'v', '1', 0, // string k1=v1

        0x08, // end struct

        0x01, 'k', '2', 0, 'v', '2', 0, // string k2=v2

        0x08, // end root

    };

    std::vector<std::pair<std::string, std::string>> kvs;

    fmt::ExtractStringKVs(kv, sizeof(kv), kvs);

    CHECK(kvs.size() == 2);

    CHECK(kvs[0].first == "k1" && kvs[0].second == "v1");

    CHECK(kvs[1].first == "k2" && kvs[1].second == "v2");

    // Truncated input stops cleanly keeping parsed-so-far pairs.

    kvs.clear();

    fmt::ExtractStringKVs(kv, 8, kvs); // cuts mid first value

    CHECK(kvs.empty());
}

void DonorPoolTest() {

    namespace dp = ac::hooks::DonorPool;

    CHECK(dp::kPoolCount == 15);

    CHECK(dp::DonorSteamId(14) == 76561198028121353ULL); // LumaCore default donor

    CHECK(dp::DonorSteamId(0) != dp::DonorSteamId(1));

    // Fresh app starts at the LumaCore default index (14).

    const ac::steam::AppId app = 999001u;

    CHECK_EQ(dp::PickIndex(app), static_cast<std::size_t>(14));

    // A data-less answer advances the round-robin.

    dp::NoteAttemptResult(app, 14, /*okWithData=*/false);

    CHECK(dp::PickIndex(app) == 0);

    // A data-bearing answer pins the donor as preferred.

    dp::NoteAttemptResult(app, 3, /*okWithData=*/true);

    CHECK(dp::PickIndex(app) == 3);

    // Pending client-stats correlation: record then take consumes it.

    dp::StatAttempt a;

    a.appId = app;

    a.poolIndex = 3;

    a.seen = dp::Clock::now();

    dp::RecordPendingClientStats(app, a);

    dp::StatAttempt out;

    CHECK(dp::TakePendingClientStats(app, out));

    CHECK(out.appId == app && out.poolIndex == 3);

    CHECK(!dp::TakePendingClientStats(app, out)); // consumed

    // Service-path correlation by jobid.

    dp::StatAttempt b;

    b.appId = app;

    b.poolIndex = 7;

    dp::RecordAttempt(b, /*hasJobId=*/true, 4242ull);

    dp::StatAttempt got;

    CHECK(dp::ResolveAttempt(true, 4242ull, got) == dp::Correl::Resolved);

    CHECK(got.appId == app && got.poolIndex == 7);

    CHECK(dp::ResolveAttempt(true, 9999ull, got) == dp::Correl::NoMatch);
}

// ---------------------------------------------------------------------------

// Batch 7-9 cases

// ---------------------------------------------------------------------------

void ManifestIdentityTest() {

    namespace mi = ac::manifestidentity;

    auto putU32 = [](std::string& s, std::uint32_t v) {
        s.push_back(static_cast<char>(v & 0xFF));

        s.push_back(static_cast<char>((v >> 8) & 0xFF));

        s.push_back(static_cast<char>((v >> 16) & 0xFF));

        s.push_back(static_cast<char>((v >> 24) & 0xFF));
    };

    auto putVarint = [](std::string& s, std::uint64_t v) {
        while (v >= 0x80) {

            s.push_back(static_cast<char>((v & 0x7F) | 0x80));

            v >>= 7;
        }

        s.push_back(static_cast<char>(v));
    };

    const std::uint32_t depot = 12345u;

    const std::uint64_t gid = 7719911223344556677ULL;

    std::string meta;

    meta.push_back(0x08);

    putVarint(meta, depot); // field 1, varint

    meta.push_back(0x10);

    putVarint(meta, gid); // field 2, varint

    meta.push_back(0x1A);

    putVarint(meta, 3);

    meta += "abc"; // field 3, length-delimited skip

    meta.push_back(0x09);

    meta.append(8, '\0'); // field 1, fixed64 skip

    meta.push_back(0x0D);

    meta.append(4, '\0'); // field 1, fixed32 skip

    std::string blob;

    putU32(blob, 0x71F617D0u);

    putU32(blob, 2);

    blob += "XX"; // header w/ payload

    putU32(blob, 0x1F4812BEu);

    putU32(blob, static_cast<std::uint32_t>(meta.size()));

    blob += meta;

    CHECK(mi::ManifestIdentityMatches(blob, depot, gid));

    CHECK(!mi::ManifestIdentityMatches(blob, depot + 1, gid));

    CHECK(!mi::ManifestIdentityMatches(blob, depot, gid + 1));

    CHECK(!mi::ManifestIdentityMatches(std::string_view(blob).substr(0, blob.size() - 1), depot, gid));

    std::string badMagic = blob;

    putU32(badMagic = badMagic.substr(0, 4), 0x12345678u);

    CHECK(!mi::ManifestIdentityMatches(badMagic, depot, gid));

    // Unknown wire type is rejected.

    std::string weirdMeta;

    weirdMeta.push_back(0x08);

    putVarint(weirdMeta, depot);

    weirdMeta.push_back(0x0B); // wire type 3: unsupported

    std::string weirdBlob;

    putU32(weirdBlob, 0x71F617D0u);

    putU32(weirdBlob, 0);

    putU32(weirdBlob, 0x1F4812BEu);

    putU32(weirdBlob, static_cast<std::uint32_t>(weirdMeta.size()));

    weirdBlob += weirdMeta;

    CHECK(!mi::ManifestIdentityMatches(weirdBlob, depot, gid));

    // Unit checks for the readers themselves.

    std::size_t off = 0;

    std::uint32_t u = 0;

    CHECK(mi::ReadU32Le(std::string_view("\x01\x00\x00\x80", 4), off, u) && u == 0x80000001u && off == 4);

    CHECK(!mi::ReadU32Le(std::string_view("\x01\x00", 2), off, u));

    off = 0;

    std::uint64_t w = 0;

    CHECK(mi::ReadVarint(std::string_view("\xAC\x02", 2), off, w) && w == 300 && off == 2);

    CHECK(!mi::ReadVarint(std::string_view("\x80", 1), off, w)); // truncated varint
}

void IpcReplyTest() {

    namespace ir = ac::hooks::ipcreply;

    std::uint8_t storage[64] = {};

    ac::steam::CUtlBuffer buf{};

    buf.memory.memory = storage;

    buf.put = 64;

    CHECK(ir::CanWrite(&buf, 64));

    CHECK(!ir::CanWrite(&buf, 65));

    CHECK(ir::Begin(&buf));

    CHECK(storage[0] == ac::constants::kIpcReplyTag);

    CHECK(ir::WriteU32(&buf, 4, 0xDEADBEEFu));

    CHECK(storage[4] == 0xEF && storage[5] == 0xBE && storage[6] == 0xAD && storage[7] == 0xDE);

    CHECK(ir::WriteU64(&buf, 8, 0x1122334455667788ULL));

    CHECK(storage[8] == 0x88 && storage[15] == 0x11);

    CHECK(ir::WriteAt(&buf, 60, storage + 4, 4)); // exact fit

    CHECK(!ir::WriteAt(&buf, 61, storage + 4, 4)); // overflow refused

    CHECK(!ir::WriteU32(&buf, 62, 1u)); // overflow refused

    CHECK(ir::WriteAt(&buf, 10, storage, 0)); // zero-byte write ok

    // Tiny buffer: nothing may be written.

    // NB: do not call it "small": the Windows SDK (rpcndr.h) defines

    // #define small char and MSVC would expand the variable name.

    std::uint8_t tiny[1] = {0};

    ac::steam::CUtlBuffer tinyBuf{};

    tinyBuf.memory.memory = tiny;

    tinyBuf.put = 0;

    CHECK(!ir::CanWrite(&tinyBuf, 1));

    CHECK(!ir::Begin(&tinyBuf));

    CHECK(!ir::WriteU32(&tinyBuf, 0, 7u));

    // Null safety.

    CHECK(!ir::CanWrite(nullptr, 1));

    CHECK(!ir::Begin(nullptr));

    CHECK(!ir::WriteAt(&buf, 0, nullptr, 4));
}

void SnapshotMergeTest() {

    namespace snap = ac::backup::snapshot;

    snap::SnapshotData d;

    snap::MergeUnlock(d, 10, 0); // baseline, unknown time

    CHECK(snap::HasUnlock(d, 10) && d.unlocks[0].unlockTime == 0);

    snap::MergeUnlock(d, 10, 1700000000u); // real time replaces 0

    CHECK(d.unlocks[0].unlockTime == 1700000000u);

    snap::MergeUnlock(d, 10, 1800000000u); // later time must NOT win

    CHECK(d.unlocks[0].unlockTime == 1700000000u);

    snap::MergeUnlock(d, 10, 1600000000u); // earlier time wins

    CHECK(d.unlocks[0].unlockTime == 1600000000u);

    snap::MergeUnlock(d, 10, 0); // 0 never overwrites real time

    CHECK(d.unlocks[0].unlockTime == 1600000000u);

    snap::MergeUnlock(d, 11, 1500000000u);

    CHECK(d.unlocks.size() == 2);

    snap::MergeStat(d, 5, 42u); // last committed value wins

    CHECK(snap::HasStat(d, 5) && d.stats[0].value == 42u);

    snap::MergeStat(d, 5, 43u);

    CHECK(d.stats[0].value == 43u);

    snap::MergeStat(d, 6, 1u);

    snap::MergeUnlock(d, 3, 1u);

    snap::SortAll(d);

    CHECK(d.unlocks[0].id == 3 && d.unlocks[1].id == 10 && d.unlocks[2].id == 11);

    CHECK(d.stats[0].id == 5 && d.stats[1].id == 6);
}

// --- QW1: TtlCache (also captures the Get-under-lock regression) ---

void IpcSpecParseTest() {

    using ac::ipcspec::MethodSpec;

    using ac::ipcspec::ParseSpecToml;

    std::unordered_map<std::string, std::uint8_t> ifaces;

    std::unordered_map<std::string, MethodSpec> methods;

    const std::string good = "[IClientUser]\n"

                             "interface_id = 5\n"

                             "[IClientUser.GetSteamID]\n"

                             "funcHash = \"0x1A2B3C4D\"\n"

                             "fencepost = \"1C\"\n"

                             "argc = 3\n";

    CHECK(ParseSpecToml(good, ifaces, methods));

    CHECK(ifaces.size() == 1 && ifaces.at("IClientUser") == 5);

    CHECK(methods.size() == 1);

    const auto it = methods.find("IClientUser::GetSteamID");

    CHECK(it != methods.end());

    CHECK(it->second.hash == 0x1A2B3C4Du);

    CHECK(it->second.fencepost == 0x1Cu);

    CHECK(it->second.argc == 3u);

    // Out-of-range interface_id: the whole interface (methods included) is dropped.

    ifaces.clear();

    methods.clear();

    CHECK(!ParseSpecToml("[IClientFriends]\ninterface_id = 999\n"

                         "[IClientFriends.GetPersonaName]\nfuncHash = \"ABCD\"\n",

                         ifaces, methods));

    // Malformed funcHash: method discarded; no valid methods → parse failed.

    ifaces.clear();

    methods.clear();

    CHECK(!ParseSpecToml("[IClientUser]\n[IClientUser.Broken]\nfuncHash = \"nothex\"\n", ifaces, methods));

    // Empty document or invalid TOML → false (existing state stays untouched).

    ifaces.clear();

    methods.clear();

    CHECK(!ParseSpecToml("", ifaces, methods));

    CHECK(!ParseSpecToml("[broken\nkey = 1", ifaces, methods));

    // funcHash without 0x prefix and minimum accepted value (1).

    ifaces.clear();

    methods.clear();

    CHECK(ParseSpecToml("[I]\n[I.M]\nfuncHash = \"1\"\n", ifaces, methods));

    CHECK(methods.at("I::M").hash == 1u);

    // Zero funcHash rejected (hash 0 = absent method, by contract).

    ifaces.clear();

    methods.clear();

    CHECK(!ParseSpecToml("[I]\n[I.M]\nfuncHash = \"0\"\n", ifaces, methods));

    // argc negativo ignorato, il metodo resta valido.

    ifaces.clear();

    methods.clear();

    CHECK(ParseSpecToml("[I]\n[I.M2]\nfuncHash = \"AB\"\nargc = -1\n", ifaces, methods));

    CHECK(methods.at("I::M2").argc == 0u);
}

// --- I-C: JsonWriter (golden test byte-per-byte del formato schema 9) ---

void ManifestContractTest() {

    namespace mi = ac::manifestidentity;

    constexpr std::uint32_t kDepot = 489831;

    constexpr std::uint64_t kGid = 4940892828028256588ull;

    const fs::path dir = CONTRACT_FIXTURES_DIR;

    const auto Read = [&](const char* name) {
        std::ifstream f(dir / name, std::ios::binary);

        CHECK(f.is_open());

        std::string body((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());

        return body;
    };

    // Valid: identity read from metadata, match only against exact expectations.

    const std::string valid = Read("valid_basic.bin");

    CHECK(mi::ManifestIdentityMatches(valid, kDepot, kGid));

    CHECK(!mi::ManifestIdentityMatches(valid, kDepot + 1, kGid));

    CHECK(!mi::ManifestIdentityMatches(valid, kDepot, kGid + 1));

    const std::string extra = Read("valid_extra_wiretypes.bin");

    CHECK(mi::ManifestIdentityMatches(extra, kDepot, kGid));

    // Malformed: all rejected (even against zero expectations).

    for (const char* bad : {"bad_magic_payload.bin", "bad_magic_metadata.bin", "truncated_payload.bin",

                            "truncated_metadata_varint.bin", "bad_wire_type.bin", "empty.bin", "garbage_short.bin"}) {

        const std::string body = Read(bad);

        CHECK(!mi::ManifestIdentityMatches(body, kDepot, kGid));

        CHECK(!mi::ManifestIdentityMatches(body, 0, 0));
    }
}
