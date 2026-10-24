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

    // ---- PlanOwnerAction (owner policy) -----------------------------------

    // A spoofed app must be attributed to the active account, never to the
    // account Steam recorded: with an owner that is not the signed-in user,
    // Steam renders "From your Steam Family library" on a Lua game nobody in
    // the family owns.
    {
        OwnerContext ctx;
        ctx.managed = true;
        ctx.activeSteamId = 222ull;
        ctx.currentLastOwner = 111ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Set);
        CHECK_EQ(plan.owner, 222ull);
        CHECK(!plan.dropRecord);
    }
    {
        // Active account still unknown (boot before login): drop the leftover,
        // which is the only safe value when Steam cannot attribute the app.
        OwnerContext ctx;
        ctx.managed = true;
        ctx.activeSteamId = 0;
        ctx.currentLastOwner = 111ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Remove);
    }
    {
        // The cosmetic opt-in still wins for a listed spoofed app.
        OwnerContext ctx;
        ctx.managed = true;
        ctx.listed = true;
        ctx.bannerEnabled = true;
        ctx.bannerSteamId = 42ull;
        ctx.activeSteamId = 222ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Set);
        CHECK_EQ(plan.owner, 42ull);
    }
    {
        // Banner enabled but no owner selected: the app still gets the active
        // account, so the bogus banner cannot render.
        OwnerContext ctx;
        ctx.managed = true;
        ctx.listed = true;
        ctx.bannerEnabled = true;
        ctx.bannerSteamId = 0;
        ctx.activeSteamId = 222ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Set);
        CHECK_EQ(plan.owner, 222ull);
    }
    {
        // An app Steam reports as family-shared was stamped at boot, before the
        // classification was known. Restore the value Steam had so its own
        // banner can name a local owner again.
        OwnerContext ctx;
        ctx.familyShared = true;
        ctx.activeSteamId = 222ull;
        ctx.recorded = true;
        ctx.record.written = 222ull;
        ctx.record.originalKnown = true;
        ctx.record.original = 111ull;
        ctx.currentLastOwner = 222ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Set);
        CHECK_EQ(plan.owner, 111ull);
        CHECK(plan.dropRecord);
    }
    {
        // Family-shared app whose manifest had no owner before Aether touched
        // it: restoring means removing our stamp.
        OwnerContext ctx;
        ctx.familyShared = true;
        ctx.activeSteamId = 222ull;
        ctx.recorded = true;
        ctx.record.written = 222ull;
        ctx.currentLastOwner = 222ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Remove);
        CHECK(plan.dropRecord);
    }
    {
        // Family-shared app Aether never touched: leave Steam's record alone.
        OwnerContext ctx;
        ctx.familyShared = true;
        ctx.activeSteamId = 222ull;
        ctx.currentLastOwner = 111ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Skip);
        CHECK(!plan.dropRecord);
    }
    {
        // Steam rewrote the manifest after our stamp: our record is obsolete.
        OwnerContext ctx;
        ctx.familyShared = true;
        ctx.recorded = true;
        ctx.record.written = 222ull;
        ctx.currentLastOwner = 99ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Skip);
        CHECK(plan.dropRecord);
    }
    {
        // No longer spoofed but still ours (Steam reports the app as owned):
        // the active account is a truthful last owner, so keep it and the
        // record.
        OwnerContext ctx;
        ctx.activeSteamId = 222ull;
        ctx.recorded = true;
        ctx.record.written = 222ull;
        ctx.record.originalKnown = true;
        ctx.record.original = 111ull;
        ctx.currentLastOwner = 222ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Skip);
        CHECK(!plan.dropRecord);
    }
    {
        // No longer spoofed and the stamp belongs to an account that is no
        // longer signed in: restore the original.
        OwnerContext ctx;
        ctx.activeSteamId = 333ull;
        ctx.recorded = true;
        ctx.record.written = 222ull;
        ctx.record.originalKnown = true;
        ctx.record.original = 111ull;
        ctx.currentLastOwner = 222ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Set);
        CHECK_EQ(plan.owner, 111ull);
        CHECK(plan.dropRecord);
    }
    {
        // No longer spoofed, our removal is still in place, there was no owner
        // before: nothing to restore.
        OwnerContext ctx;
        ctx.recorded = true;
        ctx.record.written = 0;
        ctx.currentLastOwner = 0;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Remove);
        CHECK(plan.dropRecord);
    }
    {
        // Steam rewrote an app Aether no longer spoofs: drop the stale record.
        OwnerContext ctx;
        ctx.recorded = true;
        ctx.record.written = 222ull;
        ctx.currentLastOwner = 99ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Skip);
        CHECK(plan.dropRecord);
    }
    {
        // Never managed, never written: a real Steam record stays untouched.
        OwnerContext ctx;
        ctx.currentLastOwner = 99ull;
        const OwnerPlan plan = PlanOwnerAction(ctx);
        CHECK(plan.action == OwnerAction::Skip);
        CHECK(!plan.dropRecord);
    }

    // ---- RecordAfterWrite (pre-Aether value capture) ----------------------

    {
        // First touch: the value Steam had becomes the restorable original.
        const OwnerRecord next = RecordAfterWrite(OwnerRecord{}, false, 111ull, 222ull);
        CHECK_EQ(next.written, 222ull);
        CHECK(next.originalKnown);
        CHECK_EQ(next.original, 111ull);
    }
    {
        // First touch with no owner: restoring means removing the entry.
        const OwnerRecord next = RecordAfterWrite(OwnerRecord{}, false, 0, 222ull);
        CHECK_EQ(next.written, 222ull);
        CHECK(!next.originalKnown);
    }
    {
        // Later passes over our own stamp keep the original.
        OwnerRecord previous;
        previous.written = 222ull;
        previous.originalKnown = true;
        previous.original = 111ull;
        const OwnerRecord next = RecordAfterWrite(previous, true, 222ull, 333ull);
        CHECK_EQ(next.written, 333ull);
        CHECK(next.originalKnown);
        CHECK_EQ(next.original, 111ull);
    }
    {
        // Steam rewrote the manifest: its fresh value is the better original.
        OwnerRecord previous;
        previous.written = 222ull;
        previous.originalKnown = true;
        previous.original = 111ull;
        const OwnerRecord next = RecordAfterWrite(previous, true, 99ull, 333ull);
        CHECK_EQ(next.written, 333ull);
        CHECK(next.originalKnown);
        CHECK_EQ(next.original, 99ull);
    }
    {
        // Removing the entry keeps the captured original for a later restore.
        OwnerRecord previous;
        previous.written = 222ull;
        previous.originalKnown = true;
        previous.original = 111ull;
        const OwnerRecord next = RecordAfterWrite(previous, true, 222ull, 0);
        CHECK_EQ(next.written, 0);
        CHECK(next.originalKnown);
        CHECK_EQ(next.original, 111ull);
    }
}
