#include "cases.h"

#include "core/AppManifestText.h"

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
        std::string content = "\"AppState\"\r\n{\r\n\t\"appid\"\t\t\"7\"\r\n\t\"LastOwner\"\t\t\"42\"\r\n\t\"StateFlags\""
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
}
