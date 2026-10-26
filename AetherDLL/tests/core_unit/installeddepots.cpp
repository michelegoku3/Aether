#include "cases.h"

#include "core/AppManifestText.h"
#include "core/InstalledDepotRepair.h"

#include <cstdint>
#include <functional>
#include <string>
#include <utility>
#include <vector>

namespace {

// Shape of a real appmanifest: two installed depots, one shared depot and the
// user block, so a repair must leave every one of them alone.
const char* kAcf = "\"AppState\"\n"
                   "{\n"
                   "\t\"appid\"\t\t\"4656000\"\n"
                   "\t\"Universe\"\t\t\"1\"\n"
                   "\t\"StateFlags\"\t\t\"4\"\n"
                   "\t\"buildid\"\t\t\"26188740\"\n"
                   "\t\"InstalledDepots\"\n"
                   "\t{\n"
                   "\t\t\"4656002\"\n"
                   "\t\t{\n"
                   "\t\t\t\"manifest\"\t\t\"4298777078523366725\"\n"
                   "\t\t\t\"size\"\t\t\"1779867546\"\n"
                   "\t\t\t\"dlcappid\"\t\t\"0\"\n"
                   "\t\t}\n"
                   "\t\t\"4656001\"\n"
                   "\t\t{\n"
                   "\t\t\t\"manifest\"\t\t\"1881228582594334662\"\n"
                   "\t\t\t\"size\"\t\t\"2458856530\"\n"
                   "\t\t}\n"
                   "\t}\n"
                   "\t\"SharedDepots\"\n"
                   "\t{\n"
                   "\t\t\"228990\"\n"
                   "\t\t{\n"
                   "\t\t\t\"manifest\"\t\t\"777\"\n"
                   "\t\t}\n"
                   "\t}\n"
                   "\t\"UserConfig\"\n"
                   "\t{\n"
                   "\t\t\"language\"\t\t\"english\"\n"
                   "\t}\n"
                   "}\n";

ac::appmanifest::LocalManifest Local(std::string gid, std::uint64_t modified, bool hasSize, std::uint64_t size) {
    ac::appmanifest::LocalManifest manifest;
    manifest.gid = std::move(gid);
    manifest.modified = modified;
    manifest.hasContentSize = hasSize;
    manifest.contentSize = size;
    return manifest;
}

// `backed` lists the "<depot>:<gid>" pairs that exist on disk.
std::function<bool(std::uint32_t, const std::string&)> Backed(std::vector<std::string> backed) {
    return [backed](std::uint32_t depotId, const std::string& gid) {
        const std::string key = std::to_string(depotId) + ":" + gid;
        for (const std::string& entry : backed)
            if (entry == key)
                return true;
        return false;
    };
}

std::function<std::vector<ac::appmanifest::LocalManifest>(std::uint32_t)>
Candidates(std::vector<std::pair<std::uint32_t, std::vector<ac::appmanifest::LocalManifest>>> perDepot) {
    return [perDepot](std::uint32_t depotId) {
        for (const auto& [id, manifests] : perDepot)
            if (id == depotId)
                return manifests;
        return std::vector<ac::appmanifest::LocalManifest>{};
    };
}

} // namespace

void InstalledDepotsTest() {
    using namespace ac::appmanifest;

    // ---- healthy manifest: byte-for-byte no-op ----------------------------

    {
        const RepairPlan plan = PlanInstalledDepotRepairs(
            kAcf, Backed({"4656002:4298777078523366725", "4656001:1881228582594334662"}), Candidates({}));
        CHECK(plan.repairs.empty());
        CHECK(plan.content == kAcf);
    }

    // Depots outside InstalledDepots are never considered.
    {
        const RepairPlan plan =
            PlanInstalledDepotRepairs(kAcf, Backed({"4656002:4298777078523366725", "4656001:1881228582594334662"}),
                                      Candidates({{228990, {Local("1", 1, true, 1)}}}));
        CHECK(plan.repairs.empty());
        CHECK(plan.content == kAcf);
    }

    // ---- phantom GID with a size match: the installed content's manifest --

    {
        // 4656002 is the installed depot: its size matches the 5722... file.
        const RepairPlan plan =
            PlanInstalledDepotRepairs(kAcf, Backed({"4656001:1881228582594334662"}),
                                      Candidates({{4656002,
                                                   {Local("9990000000000000001", 10, true, 123ull),
                                                    Local("5722240766019054112", 20, true, 1779867546ull)}}}));
        CHECK_EQ(plan.repairs.size(), std::size_t{1});
        CHECK_EQ(plan.repairs[0].depotId, 4656002u);
        CHECK(plan.repairs[0].declaredGid == "4298777078523366725");
        CHECK(plan.repairs[0].restoredGid == "5722240766019054112");

        // Only that one value changed; the other depot, the shared depot, the
        // build id and the user block are byte-identical.
        CHECK(plan.content.find("\"manifest\"\t\t\"5722240766019054112\"") != std::string::npos);
        CHECK(plan.content.find("4298777078523366725") == std::string::npos);
        CHECK(plan.content.find("\"size\"\t\t\"1779867546\"") != std::string::npos);
        CHECK(plan.content.find("\"manifest\"\t\t\"1881228582594334662\"") != std::string::npos);
        CHECK(plan.content.find("\"manifest\"\t\t\"777\"") != std::string::npos);
        CHECK(plan.content.find("\"buildid\"\t\t\"26188740\"") != std::string::npos);
        CHECK(plan.content.find("\"language\"\t\t\"english\"") != std::string::npos);
        CHECK_EQ(plan.content.size(), std::string(kAcf).size());
    }

    // ---- phantom GID without a size match: newest local file --------------

    {
        // The ACF size describes content no local manifest declares (a stale
        // size): pick the newest file and record its own size.
        const RepairPlan plan = PlanInstalledDepotRepairs(
            kAcf, Backed({"4656001:1881228582594334662"}),
            Candidates({{4656002,
                         {Local("1000000000000000001", 10, true, 5ull), Local("2000000000000000002", 30, true, 7ull),
                          Local("3000000000000000003", 20, true, 6ull)}}}));
        CHECK_EQ(plan.repairs.size(), std::size_t{1});
        CHECK(plan.repairs[0].restoredGid == "2000000000000000002");
        CHECK(plan.content.find("\"manifest\"\t\t\"2000000000000000002\"") != std::string::npos);
        CHECK(plan.content.find("\"size\"\t\t\"7\"") != std::string::npos);
    }

    // Equal write times: the GID breaks the tie, so the choice does not depend
    // on the directory enumeration order.
    {
        const RepairPlan a = PlanInstalledDepotRepairs(
            kAcf, Backed({"4656001:1881228582594334662"}),
            Candidates(
                {{4656002, {Local("9000000000000000009", 50, false, 0), Local("8000000000000000008", 50, false, 0)}}}));
        const RepairPlan b = PlanInstalledDepotRepairs(
            kAcf, Backed({"4656001:1881228582594334662"}),
            Candidates(
                {{4656002, {Local("8000000000000000008", 50, false, 0), Local("9000000000000000009", 50, false, 0)}}}));
        CHECK(a.repairs.size() == 1 && b.repairs.size() == 1);
        CHECK(a.repairs[0].restoredGid == "9000000000000000009");
        CHECK(b.repairs[0].restoredGid == a.repairs[0].restoredGid);
    }

    // ---- nothing to restore: leave the file untouched ---------------------

    {
        // No local manifest for that depot at all: a rewrite could only make it
        // worse, so the entry stays as Steam wrote it.
        const RepairPlan plan =
            PlanInstalledDepotRepairs(kAcf, Backed({"4656001:1881228582594334662"}), Candidates({}));
        CHECK(plan.repairs.empty());
        CHECK(plan.content == kAcf);
    }

    // A declared GID of "0" (depot registered, nothing installed) is not a
    // broken identity and must not be rewritten.
    {
        const std::string acf =
            "\"AppState\"\n{\n\t\"InstalledDepots\"\n\t{\n\t\t\"7\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"0\"\n"
            "\t\t}\n\t}\n}\n";
        const RepairPlan plan =
            PlanInstalledDepotRepairs(acf, Backed({}), Candidates({{7, {Local("123", 1, true, 1)}}}));
        CHECK(plan.repairs.empty());
        CHECK(plan.content == acf);
    }

    // No InstalledDepots block: nothing is parsed, nothing is rewritten.
    {
        const std::string acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"1\"\n}\n";
        const RepairPlan plan =
            PlanInstalledDepotRepairs(acf, Backed({}), Candidates({{1, {Local("123", 1, true, 1)}}}));
        CHECK(plan.repairs.empty());
        CHECK(plan.content == acf);
    }

    // A depot without a `size` line still gets its GID repaired.
    {
        const std::string acf =
            "\"AppState\"\n{\n\t\"InstalledDepots\"\n\t{\n\t\t\"9\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"111\"\n"
            "\t\t}\n\t}\n}\n";
        const RepairPlan plan =
            PlanInstalledDepotRepairs(acf, Backed({}), Candidates({{9, {Local("222", 1, true, 42)}}}));
        CHECK_EQ(plan.repairs.size(), std::size_t{1});
        CHECK(plan.content.find("\"manifest\"\t\t\"222\"") != std::string::npos);
        CHECK(plan.content.find("\"size\"") == std::string::npos);
    }

    // CRLF manifests keep their endings on every line.
    {
        std::string acf = kAcf;
        std::string crlf;
        for (const char c : acf)
            crlf += c == '\n' ? "\r\n" : std::string(1, c);
        acf = std::move(crlf);

        const RepairPlan plan =
            PlanInstalledDepotRepairs(acf, Backed({"4656001:1881228582594334662"}),
                                      Candidates({{4656002, {Local("5722240766019054112", 1, true, 1779867546ull)}}}));
        CHECK_EQ(plan.repairs.size(), std::size_t{1});
        CHECK_EQ(plan.content.size(), acf.size());
        CHECK(plan.content.find("\"manifest\"\t\t\"5722240766019054112\"\r\n") != std::string::npos);
        CHECK(plan.content.find("\n\n") == std::string::npos);
        for (std::size_t i = 0; i < plan.content.size(); ++i)
            CHECK(plan.content[i] != '\n' || (i > 0 && plan.content[i - 1] == '\r'));
    }

    // Only the broken depot is repaired when two entries share the block.
    {
        const RepairPlan plan =
            PlanInstalledDepotRepairs(kAcf, Backed({}),
                                      Candidates({{4656002, {Local("5722240766019054112", 1, true, 1779867546ull)}},
                                                  {4656001, {Local("1771228582594334662", 1, true, 2458856530ull)}}}));
        CHECK_EQ(plan.repairs.size(), std::size_t{2});
        CHECK(plan.content.find("\"manifest\"\t\t\"5722240766019054112\"") != std::string::npos);
        CHECK(plan.content.find("\"manifest\"\t\t\"1771228582594334662\"") != std::string::npos);
    }

    // ---- ParseInstalledDepots --------------------------------------------

    {
        const SplitContent split = SplitContentLines(kAcf);
        const std::vector<DepotEntry> entries = ParseInstalledDepots(split.lines);
        CHECK_EQ(entries.size(), std::size_t{2});

        CHECK_EQ(entries[0].depotId, 4656002u);
        CHECK(entries[0].manifestGid == "4298777078523366725");
        CHECK(entries[0].hasSize);
        CHECK_EQ(entries[0].size, 1779867546ull);
        CHECK(split.lines[entries[0].manifestLine].find("4298777078523366725") != std::string::npos);
        CHECK(split.lines[entries[0].sizeLine].find("1779867546") != std::string::npos);

        CHECK_EQ(entries[1].depotId, 4656001u);
        CHECK(entries[1].manifestGid == "1881228582594334662");
        CHECK_EQ(entries[1].size, 2458856530ull);
    }

    // Unbalanced braces (a truncated file) never yield a bogus entry.
    {
        const std::string acf =
            "\"AppState\"\n{\n\t\"InstalledDepots\"\n\t{\n\t\t\"5\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"9\"\n";
        const SplitContent split = SplitContentLines(acf);
        CHECK(ParseInstalledDepots(split.lines).empty());
    }

    // ---- Split / Join round-trip -----------------------------------------

    for (const char* sample : {"", "\n", "a", "a\n", "a\r\n", "a\r\nb\r\n", "a\r\nb", "a\n\nb\n"}) {
        const SplitContent split = SplitContentLines(sample);
        CHECK(JoinContentLines(split) == std::string(sample));
    }

    // ---- AppIdFromManifestName -------------------------------------------

    CHECK_EQ(AppIdFromManifestName("appmanifest_4656000.acf"), 4656000u);
    CHECK_EQ(AppIdFromManifestName("appmanifest_1.acf"), 1u);
    CHECK_EQ(AppIdFromManifestName("appmanifest_.acf"), 0u);
    CHECK_EQ(AppIdFromManifestName("appmanifest_12a.acf"), 0u);
    CHECK_EQ(AppIdFromManifestName("appmanifest_99999999999.acf"), 0u);
    CHECK_EQ(AppIdFromManifestName("appmanifest_1.acf.bak"), 0u);
    CHECK_EQ(AppIdFromManifestName("4656000.acf"), 0u);
    CHECK_EQ(AppIdFromManifestName(""), 0u);
}
