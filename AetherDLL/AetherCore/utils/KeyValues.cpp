#include "pch.h"
#include "utils/KeyValues.h"

#include <cstring>

namespace ac::kv1 {
namespace {

constexpr std::uint8_t kTypeDict = 0x00;
constexpr std::uint8_t kTypeString = 0x01;
constexpr std::uint8_t kTypeIntA = 0x02;
constexpr std::uint8_t kTypeIntB = 0x03;
constexpr std::uint8_t kTypeEnd = 0x08;

bool ReadCString(const std::uint8_t* data, std::size_t size, std::size_t& pos,
                 std::string& out) {
    const std::size_t start = pos;
    while (pos < size && data[pos] != 0) ++pos;
    if (pos >= size) return false;
    out.assign(reinterpret_cast<const char*>(data + start), pos - start);
    ++pos;
    return true;
}

bool WalkEntries(const std::uint8_t* data, std::size_t size, std::size_t& pos,
                 int depth, Visitor& visitor) {
    while (pos < size) {
        const std::uint8_t type = data[pos++];
        if (type == kTypeEnd) {
            return true;   // closes the current dict, or the stream at depth 0
        }
        std::string name;
        if (type == kTypeDict) {
            if (!ReadCString(data, size, pos, name)) return false;
            visitor.OnDictBegin(name, depth);
            if (!WalkEntries(data, size, pos, depth + 1, visitor)) return false;
            visitor.OnDictEnd(depth);
        } else if (type == kTypeString) {
            std::string value;
            if (!ReadCString(data, size, pos, name)) return false;
            if (!ReadCString(data, size, pos, value)) return false;
            visitor.OnString(name, value, depth);
        } else if (type == kTypeIntA || type == kTypeIntB) {
            if (!ReadCString(data, size, pos, name)) return false;
            if (size - pos < 4) return false;
            std::int32_t value = 0;
            std::memcpy(&value, data + pos, 4);
            pos += 4;
            visitor.OnInt32(name, type, value, depth);
        } else {
            return false;   // unknown type: stop, keep what was collected
        }
    }
    return true;
}

}  // namespace

bool WalkBinary(const std::uint8_t* data, std::size_t size, Visitor& visitor) {
    if (!data || size == 0) return false;
    std::size_t pos = 0;
    return WalkEntries(data, size, pos, 0, visitor);
}

std::vector<std::pair<std::string, std::string>> ReadStringKVs(const std::uint8_t* data,
                                                               std::size_t size) {
    struct Collector final : Visitor {
        std::vector<std::pair<std::string, std::string>> pairs;
        void OnString(const std::string& key, const std::string& value, int) override {
            pairs.emplace_back(key, value);
        }
    } collector;
    WalkBinary(data, size, collector);
    return collector.pairs;
}

std::string WriteStringKVs(const std::vector<std::pair<std::string, std::string>>& pairs) {
    std::string out;
    out.push_back(static_cast<char>(kTypeDict));
    out += "root";
    out.push_back('\0');
    for (const auto& [key, value] : pairs) {
        out.push_back(static_cast<char>(kTypeString));
        out += key;
        out.push_back('\0');
        out += value;
        out.push_back('\0');
    }
    out.push_back(static_cast<char>(kTypeEnd));   // close "root"
    out.push_back(static_cast<char>(kTypeEnd));   // close the stream
    return out;
}

}  // namespace ac::kv1
