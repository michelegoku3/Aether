#include "cases.h"

#include "core/AppManifestText.h"
#include "core/OwnerRecordPolicy.h"

namespace {

const char* kAcf = "\"AppState\"\n"
                   "{\n"
                   "\t\"appid\"\t\t\"4656000\"\n"
                   "\t\"name\"\t\t\"BOMBANANA!\"\n"
                   "\t\"StateFlags\"\t\t\"4\"\n"
                   "\t\"buildid\"\t\t\"25904038\"\n"
                   "\t\"LastOwner\"\t\t\"76561199876393402\"\n"
                   "\t\"BytesToDownload\"\t\t\"0\"\n"
                   "\t\"InstalledDepots\"\n"
                   "\t{\n"
                   "\t\t\"4656002\"\n"
                   "\t\t{\n"
                   "\t\t\t\"manifest\"\t\t\"9060506692183362500\"\n"
                   "\t\t}\n"
                   "\t}\n"
                   "}\n";

} // namespace

void AcfOwnerTest() {
    using namespace ac::appmanifest;

    // ---- RemoveLastOwner (default policy) ---------------------------------

    // Existing value: the whole line goes, every other byte stays.
    {
        std::string content = kAcf;
        CHECK(RemoveLastOwner(content));
        CHECK(content.find("LastOwner") == std::string::npos);
        CHECK(content.find("76561199876393402") == std::string::npos);
        CHECK(content.find("\"StateFlags\"\t\t\"4\"") != std::string::npos);
        CHECK(content.find("\"manifest\"\t\t\"9060506692183362500\"") != std::string::npos);
        CHECK(content.find("\"buildid\"\t\t\"25904038\"") != std::string::npos);
        CHECK(content.find("\"BytesToDownload\"\t\t\"0\"") != std::string::npos);
        // Only that one line disappears: tab + quoted key + two tabs + quoted
        // 17-digit id + newline = 34 bytes.
        CHECK(content.size() == std::string(kAcf).size() - 34);
    }

    // Already absent: no rewrite, content untouched.
    {
        std::string content = "\"AppState\"\n{\n\t\"appid\"\t\t\"1\"\n}\n";
        const std::string before = content;
        CHECK(!RemoveLastOwner(content));
        CHECK(content == before);
    }

    // CRLF manifests: the line including its \r\n is removed, CRLF preserved.
    {
        std::string content =
            "\"AppState\"\r\n{\r\n\t\"appid\"\t\t\"7\"\r\n\t\"LastOwner\"\t\t\"42\"\r\n\t\"StateFlags\""
            "\t\t\"4\"\r\n}\r\n";
        CHECK(RemoveLastOwner(content));
        CHECK(content == "\"AppState\"\r\n{\r\n\t\"appid\"\t\t\"7\"\r\n\t\"StateFlags\"\t\t\"4\"\r\n}\r\n");
    }

    // A nested LastOwner (depth > 1) is not the AppState owner and is ignored.
    {
        std::string content = "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n\t\"UserConfig\"\n\t{\n"
                              "\t\t\"LastOwner\"\t\t\"111\"\n\t}\n}\n";
        const std::string before = content;
        CHECK(!RemoveLastOwner(content));
        CHECK(content == before);
    }

    // Unterminated last line: removed without leaving a dangling newline.
    {
        std::string content = "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n\t\"LastOwner\"\t\t\"9\"";
        CHECK(RemoveLastOwner(content));
        CHECK(content == "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n");
    }

    // Not an AppState body and no owner entry: no change.
    {
        const std::string other = "\"OtherRoot\"\n{\n\t\"foo\"\t\t\"1\"\n}\n";
        std::string content = other;
        CHECK(!RemoveLastOwner(content));
        CHECK(content == other);
    }

    // ---- SetLastOwner (opt-in banner policy) ------------------------------

    // Existing value: rewritten in place, every other byte preserved.
    {
        std::string content = kAcf;
        CHECK(SetLastOwner(content, 76561199523209859ull));
        CHECK(content.find("\"LastOwner\"\t\t\"76561199523209859\"") != std::string::npos);
        CHECK(content.find("76561199876393402") == std::string::npos);
        CHECK(content.find("\"StateFlags\"\t\t\"4\"") != std::string::npos);
        CHECK(content.find("\"manifest\"\t\t\"9060506692183362500\"") != std::string::npos);
        CHECK(content.size() == std::string(kAcf).size());
    }

    // Already the wanted value: no rewrite, content untouched.
    {
        std::string content = "\"AppState\"\n{\n\t\"appid\"\t\t\"1\"\n\t\"LastOwner\"\t\t\"42\"\n}\n";
        const std::string before = content;
        CHECK(!SetLastOwner(content, 42ull));
        CHECK(content == before);
    }

    // Missing key: inserted right after appid with the file's line ending.
    {
        std::string content = "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n\t\"StateFlags\"\t\t\"4\"\n}\n";
        CHECK(SetLastOwner(content, 76561199523209859ull));
        CHECK(content == "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n\t\"LastOwner\"\t\t\"76561199523209859\"\n"
                         "\t\"StateFlags\"\t\t\"4\"\n}\n");
    }
    {
        std::string content = "\"AppState\"\r\n{\r\n\t\"appid\"\t\t\"7\"\r\n}\r\n";
        CHECK(SetLastOwner(content, 42ull));
        CHECK(content == "\"AppState\"\r\n{\r\n\t\"appid\"\t\t\"7\"\r\n\t\"LastOwner\"\t\t\"42\"\r\n}\r\n");
    }

    // Zero identity is never written.
    {
        const std::string body = "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n}\n";
        std::string content = body;
        CHECK(!SetLastOwner(content, 0));
        CHECK(content == body);
    }

    // Set then remove round-trips to the original body (minus the entry line).
    {
        std::string content = "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n\t\"LastOwner\"\t\t\"111\"\n}\n";
        CHECK(SetLastOwner(content, 222ull));
        CHECK(RemoveLastOwner(content));
        CHECK(content == "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n}\n");
    }

    // ---- ReadLastOwner ----------------------------------------------------

    CHECK(ReadLastOwner(kAcf) == 76561199876393402ull);
    {
        const std::string content = "\"AppState\"\n{\n\t\"appid\"\t\t\"1\"\n}\n";
        CHECK(ReadLastOwner(content) == 0);
    }
    {
        // CRLF is handled; malformed and overflowing values never parse.
        const std::string crlf = "\"AppState\"\r\n{\r\n\t\"LastOwner\"\t\t\"42\"\r\n}\r\n";
        CHECK(ReadLastOwner(crlf) == 42ull);
        const std::string bad = "\"AppState\"\n{\n\t\"LastOwner\"\t\t\"12x\"\n}\n";
        CHECK(ReadLastOwner(bad) == 0);
        const std::string overflow = "\"AppState\"\n{\n\t\"LastOwner\"\t\t\"99999999999999999999\"\n}\n";
        CHECK(ReadLastOwner(overflow) == 0);
    }
    {
        // A nested LastOwner is not the AppState owner.
        const std::string nested = "\"AppState\"\n{\n\t\"UserConfig\"\n\t{\n\t\t\"LastOwner\"\t\t\"111\"\n\t}\n}\n";
        CHECK(ReadLastOwner(nested) == 0);
    }

    // ---- PlanOwnerAction (cleanup / orphan handling) ----------------------

    {
        // Listed, managed and enabled: stamp.
        const OwnerPlan plan = PlanOwnerAction(true, true, true, 42ull, 0, 0);
        CHECK(plan.action == OwnerAction::Set);
        CHECK_EQ(plan.owner, 42ull);
        CHECK(!plan.dropRecord);
    }
    {
        // The app left the managed set but the value is still ours: clean it.
        const OwnerPlan plan = PlanOwnerAction(true, false, false, 42ull, 42ull, 42ull);
        CHECK(plan.action == OwnerAction::Remove);
    }
    {
        // The feature is off: clean what Aether wrote earlier.
        const OwnerPlan plan = PlanOwnerAction(false, true, true, 42ull, 42ull, 42ull);
        CHECK(plan.action == OwnerAction::Remove);
    }
    {
        // Steam rewrote the value (real license): drop the record, change nothing.
        const OwnerPlan plan = PlanOwnerAction(true, false, false, 42ull, 42ull, 99ull);
        CHECK(plan.action == OwnerAction::Skip);
        CHECK(plan.dropRecord);
    }
    {
        // Already gone: drop the record, nothing to write.
        const OwnerPlan plan = PlanOwnerAction(true, false, false, 42ull, 42ull, 0);
        CHECK(plan.action == OwnerAction::Skip);
        CHECK(plan.dropRecord);
    }
    {
        // Never written by Aether: never touch a real Steam record.
        const OwnerPlan plan = PlanOwnerAction(true, false, false, 42ull, 0, 99ull);
        CHECK(plan.action == OwnerAction::Skip);
        CHECK(!plan.dropRecord);
    }
    {
        // Enabled but no owner selected: fall back to the remove policy.
        const OwnerPlan plan = PlanOwnerAction(true, true, true, 0, 42ull, 42ull);
        CHECK(plan.action == OwnerAction::Remove);
    }
}
