#pragma once

// ---------------------------------------------------------------------------
// JsonWriter — declarative JSON emitter for StatusWriter (I-C).
//
// Before, WriteSnapshot concatenated ~150 hand-written lines of
// `json << "  \"key\": ...,\n"`: adding, removing or moving a field risked
// breaking TWO commas (the neighboring field's). Here punctuation (commas,
// indent, newlines) is handled ONCE by the writer; every field is a
// self-contained call.
//
// Byte-for-byte compatibility: the format reproduces exactly the published
// schema_version 9 (2-space indent, arrays on their own lines, last field
// without comma, "}\n" close). The "jsonwriter" suite in quickwin_tests pins
// it with a golden test.
// ---------------------------------------------------------------------------

#include <cstddef>
#include <string>
#include <vector>

namespace ac::jsonw {

// Identical escape to StatusWriter's old EscapeJson (ASCII expected;
// quote/backslash/newline and <0x20 controls -> \u00XX).
inline std::string Escape(const std::string& in) {
    std::string out;
    out.reserve(in.size() + 2);
    for (char c : in) {
        switch (c) {
        case '"':
            out += "\\\"";
            break;
        case '\\':
            out += "\\\\";
            break;
        case '\n':
            out += "\\n";
            break;
        case '\r':
            out += "\\r";
            break;
        case '\t':
            out += "\\t";
            break;
        default:
            if (static_cast<unsigned char>(c) < 0x20) {
                constexpr char hex[] = "0123456789abcdef";
                out += "\\u00";
                out += hex[(static_cast<unsigned char>(c) >> 4) & 15];
                out += hex[static_cast<unsigned char>(c) & 15];
            } else
                out += c;
            break;
        }
    }
    return out;
}

class Writer {
  public:
    Writer() { out_ += "{\n"; }

    void Int64(const char* key, long long value) { Field(key, std::to_string(value)); }
    void UInt64(const char* key, unsigned long long value) { Field(key, std::to_string(value)); }
    void SizeT(const char* key, std::size_t value) {
        Field(key, std::to_string(static_cast<unsigned long long>(value)));
    }
    void Bool(const char* key, bool value) { Field(key, value ? "true" : "false"); }
    void Str(const char* key, const std::string& value) { Field(key, "\"" + Escape(value) + "\""); }

    // String array: empty [] inline, otherwise one element per line (4 spaces),
    // "\n  ]" close — like the schema 9 hooks_*_list.
    void StrArray(const char* key, const std::vector<std::string>& items) {
        std::string body = "[";
        for (std::size_t i = 0; i < items.size(); ++i) {
            body += (i == 0 ? "\n    " : ",\n    ");
            body += "\"";
            body += Escape(items[i]);
            body += "\"";
        }
        if (!items.empty())
            body += "\n  ";
        body += "]";
        Field(key, body);
    }

    // Array of already-rendered inline objects ("{...}" element): used by
    // diagnostics. ObjectArray opens, RawObject appends, ArrayEnd closes.
    void ObjectArray(const char* key) {
        Separator();
        out_ += "  \"";
        out_ += key;
        out_ += "\": [";
        arrayItems_ = 0;
    }
    void RawObject(const std::string& inlineObject) {
        out_ += (arrayItems_ == 0 ? "\n    " : ",\n    ");
        out_ += inlineObject;
        ++arrayItems_;
    }
    void ArrayEnd() {
        if (arrayItems_ != 0)
            out_ += "\n  ";
        out_ += "]";
    }

    // Closes the object: the last field stays WITHOUT comma; the final line is
    // "\n}\n" (newline before the brace, like the old emitter).
    std::string Finish() {
        out_ += "\n}\n";
        return std::move(out_);
    }

  private:
    void Separator() {
        if (!first_)
            out_ += ",\n";
        first_ = false;
    }
    void Field(const char* key, const std::string& renderedValue) {
        Separator();
        out_ += "  \"";
        out_ += key;
        out_ += "\": ";
        out_ += renderedValue;
    }

    std::string out_;
    bool first_ = true;
    std::size_t arrayItems_ = 0;
};

} // namespace ac::jsonw
