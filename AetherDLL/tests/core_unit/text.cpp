#include "cases.h"

#include "credentials/HexCodec.h"

#include "utils/JsonStringField.h"

#include "utils/JsonWriter.h"

#include "utils/KeyValues.h"

#include "utils/SignatureCodec.h"

#include "utils/Strings.h"

#include "utils/VdfText.h"

void Strings() {

    using namespace ac::strings;

    CHECK(EqualsIgnoreCase("EXAMPLE.com", "example.COM"));

    CHECK(!EqualsIgnoreCase("ab", "abc"));

    CHECK(Trim(" \t\r\n foo bar \t") == "foo bar");

    CHECK(Trim(" \t\r\n").empty());

    CHECK(ExtractHost("HTTPS://Example.com:443/path?q=a@b") == "Example.com");

    CHECK(ExtractHost("http://127.0.0.1/test") == "127.0.0.1");

    for (const auto* bad :

         {"https://u:p@host/x", "https://host:443@evil/", "https://host\\evil/", "https://host:99999999999999/",

          "https://host:0/", "https://host:/", "https://host:abc/", "https://host:1:2/", "https://[::1]/",

          "https://host name/", "https:///x", "ftp://host/", "host/x", ""})

        CHECK(ExtractHost(bad).empty());

#ifdef _WIN32

    CHECK(Widen("calf\xc3\xa9") == L"calf\u00e9");

    CHECK(Widen("\xff").empty());

#endif
}

void KeyValuesTest() {

    namespace kv1 = ac::kv1;

    // Blob: root { s1 { str "hello"; i32 = 7 }  str "world"; end; end }

    const std::uint8_t blob[] = {

        0x00, 's', '1', 0, // dict "s1"

        0x01, 'k', 'e', 'y', 0, 'h', 'e', 'l', 'l', 'o', 0, // string

        0x02, 'n', 0,   7,   0, 0,   0, // int32

        0x08, // end s1

        0x01, 't', 'o', 'p', 0, 'w', 'o', 'r', 'l', 'd', 0, // string

        0x08, // end root

    };

    // Visitor: depth + callback order.

    struct Rec final : kv1::Visitor {

        std::vector<std::string> events;

        void OnDictBegin(const std::string& n, int d) override { events.push_back("D+" + std::to_string(d) + ":" + n); }

        void OnDictEnd(int d) override { events.push_back("D-" + std::to_string(d)); }

        void OnString(const std::string& k, const std::string& v, int d) override {

            events.push_back("S" + std::to_string(d) + ":" + k + "=" + v);
        }

        void OnInt32(const std::string& k, std::uint8_t, std::int32_t v, int d) override {

            events.push_back("I" + std::to_string(d) + ":" + k + "=" + std::to_string(v));
        }

    } rec;

    CHECK(kv1::WalkBinary(blob, sizeof(blob), rec));

    CHECK(rec.events.size() == 5);

    CHECK(rec.events[0] == "D+0:s1");

    CHECK(rec.events[1] == "S1:key=hello");

    CHECK(rec.events[2] == "I1:n=7");

    CHECK(rec.events[3] == "D-0");

    CHECK(rec.events[4] == "S0:top=world");

    // ReadStringKVs: flat pairs at any depth.

    auto pairs = kv1::ReadStringKVs(blob, sizeof(blob));

    CHECK(pairs.size() == 2);

    CHECK(pairs[0].first == "key" && pairs[0].second == "hello");

    CHECK(pairs[1].first == "top" && pairs[1].second == "world");

    // Writer round-trip.

    const std::string written = kv1::WriteStringKVs({{"a", "1"}, {"b", "two"}});

    auto back = kv1::ReadStringKVs(reinterpret_cast<const std::uint8_t*>(written.data()), written.size());

    CHECK(back.size() == 2 && back[0].first == "a" && back[0].second == "1" && back[1].first == "b" &&

          back[1].second == "two");

    // Malformed: unknown type -> false.

    const std::uint8_t bad[] = {0x77, 'x', 0};

    struct Empty final : kv1::Visitor {

    } empty;

    CHECK(!kv1::WalkBinary(bad, sizeof(bad), empty));

    // Truncated string -> false.

    const std::uint8_t trunc[] = {0x01, 'k', 0, 'v'};

    CHECK(!kv1::WalkBinary(trunc, sizeof(trunc), empty));
}

void VdfTextTest() {

    namespace vdf = ac::vdf;

    std::string k, v;

    CHECK(vdf::ParseLine("\t\"apps\" \t ", k, v) == vdf::LineKind::SectionKey && k == "apps");

    CHECK(vdf::ParseLine("\"LastPlayed\" \t \"1700000000\"", k, v) == vdf::LineKind::KeyValue && k == "LastPlayed" &&

          v == "1700000000");

    CHECK(vdf::ParseLine("{", k, v) == vdf::LineKind::None);

    CHECK(vdf::ParseLine("\"key\" {", k, v) == vdf::LineKind::None);

    CHECK(vdf::ParseLine("garbage", k, v) == vdf::LineKind::None);

    // Unterminated quote.

    CHECK(vdf::ParseLine("\"open", k, v) == vdf::LineKind::None);

    std::size_t pos = 0;

    char brace = 0;

    CHECK(vdf::FindUnquotedBrace("\"quoted { brace\"", pos, brace) == false);

    CHECK(vdf::FindUnquotedBrace("\"a\" \"b\" }", pos, brace) && brace == '}');

    CHECK(vdf::FindUnquotedBrace("x { y", pos, brace) && brace == '{' && pos == 2);

    std::string esc = "C:\\\\Games\\\\Steam"; // VDF-escaped C:\Games\Steam

    vdf::UnescapeBackslashes(esc);

    CHECK(esc == "C:\\Games\\Steam");

    const std::string content = "\"libraryfolders\"\n{\n"

                                "  \"0\"\n  {\n"

                                "    \"path\"  \"C:\\\\Steam\"\n"

                                "    \"PATH\"  \"D:\\\\Games\"\n"

                                "  }\n}\n";

    const auto paths = vdf::ExtractQuotedValues(content, "path"); // case-insensitive

    CHECK(paths.size() == 2);

    CHECK(paths[0] == "C:\\Steam");

    CHECK(paths[1] == "D:\\Games");
}

void JsonEscapesTest() {

    namespace ju = ac::jsonutil;

    std::string out;

    CHECK(ju::PullEscapedStringField("{\"key\": \"value\"}", "key", out) && out == "value");

    CHECK(ju::PullEscapedStringField("{\"key\": \"a\\\"b\\\\c\\/d\\n\\t\\r\\b\\f\"}", "key", out) &&

          out == "a\"b\\c/d\n\t\r\b\f");

    CHECK(!ju::PullEscapedStringField("{\"key\": \"bad\\u0041\"}", "key", out)); // \u rejected

    CHECK(!ju::PullEscapedStringField("{\"key\": \"bad\\q\"}", "key", out)); // unknown rejected

    CHECK(ju::PullEscapedStringField("{\"key\": \"\"}", "key", out) && out.empty()); // empty ok

    CHECK(!ju::PullEscapedStringField("{\"other\": \"x\"}", "key", out)); // missing

    CHECK(!ju::PullEscapedStringField("{\"key\": \"unterminated", "key", out));

    CHECK(!ju::PullEscapedStringField("{\"key\": 42}", "key", out)); // not a string

    // Whitespace around colon is tolerated.

    CHECK(ju::PullEscapedStringField("{\"key\" :\n\"v\"}", "key", out) && out == "v");
}

void HexCodecTest() {

    using namespace ac::hex;

    const std::vector<std::uint8_t> blob = {0x48, 0x8B, 0x05, 0x00, 0xFF};

    const std::string hex = Encode(blob);

    CHECK(hex == "488B0500FF"); // Encode is uppercase

    CHECK(Decode(hex) == blob);

    CHECK(Decode("488b05") == std::vector<std::uint8_t>({0x48, 0x8B, 0x05})); // lowercase ok

    const auto empty = Decode("");

    CHECK(empty.has_value() && empty->empty());

    CHECK(!Decode("abc").has_value()); // odd length

    CHECK(!Decode("zz").has_value()); // non-hex character

    CHECK(!Decode(" 48").has_value()); // space is not hex

    CHECK(!Decode("0x48").has_value()); // no prefixes
}

// --- QW8: SignatureCodec (parser signature estratto da PatternEngine) ---

void SignatureTest() {

    using ac::pattern::ParseSignature;

    std::vector<std::uint8_t> bytes;

    std::string mask;

    CHECK(ParseSignature("48 8B ?? C3", bytes, mask));

    CHECK(bytes.size() == 4 && mask.size() == 4);

    CHECK(mask == "xx?x");

    CHECK(bytes[0] == 0x48 && bytes[1] == 0x8B && bytes[3] == 0xC3);

    CHECK(ParseSignature("00", bytes, mask)); // 0x00 is a valid byte

    CHECK(bytes.size() == 1 && mask == "x" && bytes[0] == 0x00);

    CHECK(ParseSignature("a1", bytes, mask)); // lowercase accepted

    CHECK(bytes.size() == 1 && bytes[0] == 0xA1);

    CHECK(!ParseSignature("", bytes, mask)); // empty

    CHECK(!ParseSignature("???", bytes, mask)); // malformed wildcard

    CHECK(!ParseSignature("4Z", bytes, mask)); // non-hex digit

    CHECK(!ParseSignature("1FF", bytes, mask)); // overlong token
}

// --- QW8: IpcSpecParse (parser TOML puro estratto da IpcSpec) ---

void JsonWriterTest() {

    using ac::jsonw::Writer;

    // Copertura escape carattere-per-carattere.

    CHECK(ac::jsonw::Escape(std::string("\"\\\n\r\t\x01") + "A") == "\\\"\\\\\\n\\r\\t\\u0001A");

    CHECK(ac::jsonw::Escape("plain ascii 123") == "plain ascii 123");

    Writer w;

    w.Int64("schema_version", 9);

    w.Int64("ts", 1727950000LL);

    w.Str("build_id", std::string("b\"q\\s\nx") + '\x01' + "z");

    w.Bool("flag_t", true);

    w.Bool("flag_f", false);

    w.StrArray("empty_list", {});

    w.StrArray("names", {"alpha", "b\"eta"});

    w.ObjectArray("empty_objs");

    w.ArrayEnd();

    w.ObjectArray("diagnostics");

    w.RawObject("{\"ts_ms\": 1, \"category\": \"" + ac::jsonw::Escape("wire_eresult") + "\", \"detail\": \"" +

                ac::jsonw::Escape("a=b") + "\"}");

    w.RawObject("{\"ts_ms\": 2, \"category\": \"" + ac::jsonw::Escape("x") + "\", \"detail\": \"" +

                ac::jsonw::Escape("y") + "\"}");

    w.ArrayEnd();

    const std::string out = w.Finish();

    // Golden: EXACTLY the format published by schema_version 9

    // (2-space indent, arrays on their own lines, last field without comma).

    const std::string golden = "{\n"

                               "  \"schema_version\": 9,\n"

                               "  \"ts\": 1727950000,\n"

                               "  \"build_id\": \"b\\\"q\\\\s\\nx\\u0001z\",\n"

                               "  \"flag_t\": true,\n"

                               "  \"flag_f\": false,\n"

                               "  \"empty_list\": [],\n"

                               "  \"names\": [\n"

                               "    \"alpha\",\n"

                               "    \"b\\\"eta\"\n"

                               "  ],\n"

                               "  \"empty_objs\": [],\n"

                               "  \"diagnostics\": [\n"

                               "    {\"ts_ms\": 1, \"category\": \"wire_eresult\", \"detail\": \"a=b\"},\n"

                               "    {\"ts_ms\": 2, \"category\": \"x\", \"detail\": \"y\"}\n"

                               "  ]\n"

                               "}\n";

    CHECK(out == golden);
}

// --- I-E: DLL<->Desk contract fixture for manifest identity (D4) ---

// The SAME blobs in Tools/contract_fixtures/manifest are asserted here and in

// AetherDesk/src-tauri/src/tests/manifest_contract_tests.rs: when either side

// changes its verdict, one of the two tests breaks.
