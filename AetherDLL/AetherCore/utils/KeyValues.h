#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <utility>
#include <vector>

// ---------------------------------------------------------------------------
// Steam binary KeyValues (KV1) — the single shared reader/writer (P11).
//
// Byte format: entry stream, each entry = type byte + NUL-terminated name:
//   0x00 dict  (children follow, closed by 0x08)
//   0x01 string value
//   0x02/0x03 int32 value
//   0x08 end of the current dict (top-level 0x08 ends the stream)
// Callers used to carry private copies of this walk (rich-presence frames,
// achievement schema buckets); every new consumer goes through here so a
// malformed-input fix lands in ONE place.
// ---------------------------------------------------------------------------
namespace ac::kv1 {

// Streaming visitor; depths count the entry's own level (top level = 0,
// entries inside the first dict = 1, ...). Default callbacks do nothing.
struct Visitor {
    virtual ~Visitor() = default;
    virtual void OnDictBegin(const std::string& name, int depth) { (void)name; (void)depth; }
    virtual void OnDictEnd(int depth) { (void)depth; }
    virtual void OnString(const std::string& key, const std::string& value, int depth) {
        (void)key; (void)value; (void)depth;
    }
    virtual void OnInt32(const std::string& key, std::uint8_t type, std::int32_t value, int depth) {
        (void)key; (void)type; (void)value; (void)depth;
    }
};

// Walks the buffer once. Returns false when the stream is malformed
// (truncated name/value, unknown type) — parsing stops at that point and the
// callbacks keep everything collected so far.
bool WalkBinary(const std::uint8_t* data, std::size_t size, Visitor& visitor);

// Flat string pairs at any depth (the historical rich-presence use case).
std::vector<std::pair<std::string, std::string>> ReadStringKVs(const std::uint8_t* data,
                                                               std::size_t size);

// Writer for a single flat dict of string pairs:
//   0x00 "root" { 0x01 k1 v1 ... } 0x08 0x08
// Exists for round-trip tests and future emit paths; pairs keep their order.
std::string WriteStringKVs(const std::vector<std::pair<std::string, std::string>>& pairs);

}  // namespace ac::kv1
