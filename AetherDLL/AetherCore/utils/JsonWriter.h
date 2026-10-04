#pragma once

// ---------------------------------------------------------------------------
// JsonWriter — emitter JSON dichiarativo per StatusWriter (I-C).
//
// Prima WriteSnapshot concatenava ~150 righe di `json << "  \"key\": ...,\n"`
// a mano: aggiungere, togliere o spostare un campo rischiava di rompere DUE
// virgole (quella del campo vicino). Qui la punteggiatura (virgole, indent,
// newline) è gestita UNA volta sola dal writer; ogni campo è una chiamata
// autoconclusiva.
//
// Compatibilità byte-per-byte: il formato riproduce esattamente lo
// schema_version 9 pubblicato (2 spazi di indent, array su righe proprie,
// ultimo campo senza virgola, chiusura "}\n"). La suite "jsonwriter" di
// quickwin_tests lo fissa con un golden test.
// ---------------------------------------------------------------------------

#include <cstddef>
#include <string>
#include <vector>

namespace ac::jsonw {

// Escape identico al vecchio EscapeJson di StatusWriter (ASCII previsto;
// quote/backslash/newline e controlli <0x20 -> \u00XX).
inline std::string Escape(const std::string& in) {
    std::string out;
    out.reserve(in.size() + 2);
    for (char c : in) {
        switch (c) {
            case '"':  out += "\\\""; break;
            case '\\': out += "\\\\"; break;
            case '\n': out += "\\n";  break;
            case '\r': out += "\\r";  break;
            case '\t': out += "\\t";  break;
            default:
                if (static_cast<unsigned char>(c) < 0x20) {
                    constexpr char hex[] = "0123456789abcdef";
                    out += "\\u00";
                    out += hex[(static_cast<unsigned char>(c) >> 4) & 15];
                    out += hex[static_cast<unsigned char>(c) & 15];
                } else out += c;
                break;
        }
    }
    return out;
}

class Writer {
public:
    Writer() { out_ += "{\n"; }

    void Int64(const char* key, long long value) {
        Field(key, std::to_string(value));
    }
    void UInt64(const char* key, unsigned long long value) {
        Field(key, std::to_string(value));
    }
    void SizeT(const char* key, std::size_t value) {
        Field(key, std::to_string(static_cast<unsigned long long>(value)));
    }
    void Bool(const char* key, bool value) {
        Field(key, value ? "true" : "false");
    }
    void Str(const char* key, const std::string& value) {
        Field(key, "\"" + Escape(value) + "\"");
    }

    // Array di stringhe: [] vuoto inline, altrimenti un elemento per riga
    // (4 spazi), chiusura "\n  ]" — come le hooks_*_list dello schema 9.
    void StrArray(const char* key, const std::vector<std::string>& items) {
        std::string body = "[";
        for (std::size_t i = 0; i < items.size(); ++i) {
            body += (i == 0 ? "\n    " : ",\n    ");
            body += "\"";
            body += Escape(items[i]);
            body += "\"";
        }
        if (!items.empty()) body += "\n  ";
        body += "]";
        Field(key, body);
    }

    // Array di oggetti già renderizzati inline (elemento "{...}"): usato dai
    // diagnostics. ObjectArray apre, RawObject aggiunge, ArrayEnd chiude.
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
        if (arrayItems_ != 0) out_ += "\n  ";
        out_ += "]";
    }

    // Chiude l'oggetto: l'ultimo campo resta SENZA virgola; la riga finale è
    // "\n}\n" ( newline prima della graffa, come il vecchio emitter ).
    std::string Finish() {
        out_ += "\n}\n";
        return std::move(out_);
    }

private:
    void Separator() {
        if (!first_) out_ += ",\n";
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

}  // namespace ac::jsonw
