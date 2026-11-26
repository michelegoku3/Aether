//! Lossless Steam binary KeyValues (KV1) parser and serializer.
//!
//! Preserves exact entry order, dictionary nesting, trailing root terminators,
//! and all typed values (`Dict`, `String`, `Int32`, `Float32`, `Ptr`, `WString`,
//! `Color`, `UInt64`) so patching achievement buckets never drops non-achievement
//! stat counters or unknown Valve fields.

/// Ordered key-value entries inside a binary KV1 dictionary.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KvDict {
    pub entries: Vec<(String, KvValue)>,
}

/// Typed value inside a Steam binary KV1 stream.
#[derive(Debug, Clone, PartialEq)]
pub enum KvValue {
    Dict(KvDict),     // 0x00
    Str(String),      // 0x01
    Int32(i32),       // 0x02
    Float32(f32),     // 0x03
    Ptr(u32),         // 0x04
    WString(Vec<u8>), // 0x05 (UTF-16LE bytes up to NUL pair, kept lossless)
    Color([u8; 4]),   // 0x06
    UInt64(u64),      // 0x07
}

/// Top-level parsed KV1 document (`root` entries plus whether an extra
/// outer `0x08` end marker terminated the top-level stream, as Steam writes).
#[derive(Debug, Clone, PartialEq)]
pub struct KvDocument {
    pub root: KvDict,
    pub has_outer_end: bool,
}

impl KvDict {
    pub fn get(&self, key: &str) -> Option<&KvValue> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_ci(&self, key: &str) -> Option<&KvValue> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn get_dict(&self, key: &str) -> Option<&KvDict> {
        match self.get(key) {
            Some(KvValue::Dict(dict)) => Some(dict),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn get_dict_mut(&mut self, key: &str) -> Option<&mut KvDict> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                KvValue::Dict(dict) => Some(dict),
                _ => None,
            })
    }

    pub fn get_or_insert_dict_mut(&mut self, key: &str) -> &mut KvDict {
        if let Some(idx) = self.entries.iter().position(|(k, _)| k == key) {
            if !matches!(self.entries[idx].1, KvValue::Dict(_)) {
                self.entries[idx].1 = KvValue::Dict(KvDict::default());
            }
            return match &mut self.entries[idx].1 {
                KvValue::Dict(dict) => dict,
                _ => unreachable!(),
            };
        }
        self.entries
            .push((key.to_string(), KvValue::Dict(KvDict::default())));
        match &mut self.entries.last_mut().expect("just pushed").1 {
            KvValue::Dict(dict) => dict,
            _ => unreachable!(),
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: KvValue) {
        let key = key.into();
        if let Some(entry) = self.entries.iter_mut().find(|(k, _)| k == &key) {
            entry.1 = value;
        } else {
            self.entries.push((key, value));
        }
    }

    #[cfg(test)]
    pub fn remove(&mut self, key: &str) -> Option<KvValue> {
        self.entries
            .iter()
            .position(|(k, _)| k == key)
            .map(|idx| self.entries.remove(idx).1)
    }
}

const MAX_KV1_DEPTH: usize = 64;
const TYPE_DICT: u8 = 0x00;
const TYPE_STR: u8 = 0x01;
const TYPE_INT32: u8 = 0x02;
const TYPE_FLOAT32: u8 = 0x03;
const TYPE_PTR: u8 = 0x04;
const TYPE_WSTRING: u8 = 0x05;
const TYPE_COLOR: u8 = 0x06;
const TYPE_UINT64: u8 = 0x07;
const TYPE_END: u8 = 0x08;
const TYPE_END_ALT: u8 = 0x0B;

fn read_cstr(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let rel = data[*pos..]
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| format!("unterminated KV1 string at offset {}", *pos))?;
    let s = String::from_utf8_lossy(&data[*pos..*pos + rel]).into_owned();
    *pos += rel + 1;
    Ok(s)
}

fn read_wstring(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, String> {
    let mut cursor = *pos;
    while cursor + 1 < data.len() {
        if data[cursor] == 0 && data[cursor + 1] == 0 {
            let bytes = data[*pos..cursor].to_vec();
            *pos = cursor + 2;
            return Ok(bytes);
        }
        cursor += 2;
    }
    Err(format!("unterminated KV1 wide string at offset {}", *pos))
}

fn read_fixed<const N: usize>(data: &[u8], pos: &mut usize) -> Result<[u8; N], String> {
    if *pos + N > data.len() {
        return Err(format!("truncated KV1 fixed value at offset {}", *pos));
    }
    let mut buf = [0u8; N];
    buf.copy_from_slice(&data[*pos..*pos + N]);
    *pos += N;
    Ok(buf)
}

fn parse_dict(data: &[u8], pos: &mut usize, depth: usize) -> Result<(KvDict, bool), String> {
    if depth > MAX_KV1_DEPTH {
        return Err(format!("KV1 nesting exceeded {MAX_KV1_DEPTH} levels"));
    }
    let mut dict = KvDict::default();
    while *pos < data.len() {
        let tag = data[*pos];
        *pos += 1;
        if tag == TYPE_END || tag == TYPE_END_ALT {
            return Ok((dict, true));
        }
        let key = read_cstr(data, pos)?;
        let value = match tag {
            TYPE_DICT => {
                let (child, closed) = parse_dict(data, pos, depth + 1)?;
                if !closed {
                    return Err(format!("unclosed KV1 dictionary `{key}`"));
                }
                KvValue::Dict(child)
            }
            TYPE_STR => KvValue::Str(read_cstr(data, pos)?),
            TYPE_INT32 => KvValue::Int32(i32::from_le_bytes(read_fixed::<4>(data, pos)?)),
            TYPE_FLOAT32 => KvValue::Float32(f32::from_le_bytes(read_fixed::<4>(data, pos)?)),
            TYPE_PTR => KvValue::Ptr(u32::from_le_bytes(read_fixed::<4>(data, pos)?)),
            TYPE_WSTRING => KvValue::WString(read_wstring(data, pos)?),
            TYPE_COLOR => KvValue::Color(read_fixed::<4>(data, pos)?),
            TYPE_UINT64 => KvValue::UInt64(u64::from_le_bytes(read_fixed::<8>(data, pos)?)),
            other => {
                return Err(format!(
                    "unknown KV1 tag 0x{other:02x} for key `{key}` at offset {}",
                    *pos
                ));
            }
        };
        dict.entries.push((key, value));
    }
    Ok((dict, false))
}

pub fn parse_binary(data: &[u8]) -> Result<KvDocument, String> {
    if data.is_empty() {
        return Err("empty KV1 stream".to_string());
    }
    let mut pos = 0;
    let (root, has_outer_end) = parse_dict(data, &mut pos, 0)?;
    if root.entries.is_empty() {
        return Err("KV1 stream contained no root entries".to_string());
    }
    Ok(KvDocument {
        root,
        has_outer_end,
    })
}

fn write_cstr(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
}

fn write_dict(out: &mut Vec<u8>, dict: &KvDict) {
    for (key, value) in &dict.entries {
        match value {
            KvValue::Dict(child) => {
                out.push(TYPE_DICT);
                write_cstr(out, key);
                write_dict(out, child);
                out.push(TYPE_END);
            }
            KvValue::Str(s) => {
                out.push(TYPE_STR);
                write_cstr(out, key);
                write_cstr(out, s);
            }
            KvValue::Int32(v) => {
                out.push(TYPE_INT32);
                write_cstr(out, key);
                out.extend_from_slice(&v.to_le_bytes());
            }
            KvValue::Float32(v) => {
                out.push(TYPE_FLOAT32);
                write_cstr(out, key);
                out.extend_from_slice(&v.to_le_bytes());
            }
            KvValue::Ptr(v) => {
                out.push(TYPE_PTR);
                write_cstr(out, key);
                out.extend_from_slice(&v.to_le_bytes());
            }
            KvValue::WString(bytes) => {
                out.push(TYPE_WSTRING);
                write_cstr(out, key);
                out.extend_from_slice(bytes);
                out.extend_from_slice(&[0, 0]);
            }
            KvValue::Color(rgba) => {
                out.push(TYPE_COLOR);
                write_cstr(out, key);
                out.extend_from_slice(rgba);
            }
            KvValue::UInt64(v) => {
                out.push(TYPE_UINT64);
                write_cstr(out, key);
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
}

pub fn serialize_binary(doc: &KvDocument) -> Vec<u8> {
    let mut out = Vec::new();
    write_dict(&mut out, &doc.root);
    if doc.has_outer_end {
        out.push(TYPE_END);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_cstr(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }

    fn build_sample_stats_bin() -> Vec<u8> {
        let mut raw = Vec::new();
        // 0x00 "cache"
        raw.push(0x00);
        push_cstr(&mut raw, "cache");
        // 0x02 "crc" = 0
        raw.push(0x02);
        push_cstr(&mut raw, "crc");
        raw.extend_from_slice(&0i32.to_le_bytes());
        // 0x02 "PendingChanges" = 1
        raw.push(0x02);
        push_cstr(&mut raw, "PendingChanges");
        raw.extend_from_slice(&1i32.to_le_bytes());
        // bucket "0": high-bit achievement mask (bit 31 + bit 0 => 0x8000_0001u32)
        raw.push(0x00);
        push_cstr(&mut raw, "0");
        raw.push(0x02);
        push_cstr(&mut raw, "data");
        raw.extend_from_slice(&(0x8000_0001u32 as i32).to_le_bytes());
        raw.push(0x02);
        push_cstr(&mut raw, "pendingbits");
        raw.extend_from_slice(&1i32.to_le_bytes());
        raw.push(0x02);
        push_cstr(&mut raw, "state");
        raw.extend_from_slice(&2i32.to_le_bytes());
        raw.push(0x00);
        push_cstr(&mut raw, "AchievementTimes");
        raw.push(0x02);
        push_cstr(&mut raw, "0");
        raw.extend_from_slice(&1_732_660_000i32.to_le_bytes());
        raw.push(0x02);
        push_cstr(&mut raw, "31");
        raw.extend_from_slice(&1_732_660_123i32.to_le_bytes());
        raw.push(0x08); // close AchievementTimes
        raw.push(0x08); // close bucket 0
                        // bucket "7": float stat counter (must survive byte-for-byte!)
        raw.push(0x00);
        push_cstr(&mut raw, "7");
        raw.push(0x03);
        push_cstr(&mut raw, "data");
        raw.extend_from_slice(&12.5f32.to_le_bytes());
        raw.push(0x07);
        push_cstr(&mut raw, "extra_u64");
        raw.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        raw.push(0x08); // close bucket 7
        raw.push(0x08); // close cache
        raw.push(0x08); // outer end (Steam format)
        raw
    }

    #[test]
    fn round_trip_preserves_cache_and_unrelated_stat_buckets_byte_for_byte() {
        let raw = build_sample_stats_bin();
        let doc = parse_binary(&raw).expect("valid KV1 stream must parse");
        assert!(doc.has_outer_end);
        let cache = doc.root.get_dict("cache").expect("cache dict present");
        let b0 = cache.get_dict("0").expect("bucket 0 present");
        assert_eq!(b0.get("data"), Some(&KvValue::Int32(0x8000_0001u32 as i32)));
        let b7 = cache.get_dict("7").expect("bucket 7 present");
        assert_eq!(b7.get("data"), Some(&KvValue::Float32(12.5)));
        assert_eq!(
            b7.get("extra_u64"),
            Some(&KvValue::UInt64(0x0102_0304_0506_0708))
        );

        let reencoded = serialize_binary(&doc);
        assert_eq!(
            reencoded, raw,
            "serializer must reproduce exact input bytes"
        );
    }

    #[test]
    fn parses_without_outer_end_and_preserves_all_primitive_types() {
        let doc = KvDocument {
            root: KvDict {
                entries: vec![(
                    "root".to_string(),
                    KvValue::Dict(KvDict {
                        entries: vec![
                            ("s".to_string(), KvValue::Str("abc".to_string())),
                            ("i".to_string(), KvValue::Int32(-42)),
                            ("f".to_string(), KvValue::Float32(3.5)),
                            ("p".to_string(), KvValue::Ptr(0xDEAD_BEEF)),
                            ("c".to_string(), KvValue::Color([1, 2, 3, 255])),
                            ("w".to_string(), KvValue::WString(vec![b'A', 0, b'B', 0])),
                            ("u".to_string(), KvValue::UInt64(999_999_999_999)),
                        ],
                    }),
                )],
            },
            has_outer_end: false,
        };
        let encoded = serialize_binary(&doc);
        let decoded = parse_binary(&encoded).expect("roundtrip must parse");
        assert_eq!(decoded, doc);
    }

    #[test]
    fn rejects_truncated_and_unknown_type_streams() {
        assert!(parse_binary(&[]).is_err());
        // Unknown type byte 0x0A
        assert!(parse_binary(&[0x0A, b'k', 0x00]).is_err());
        // Int32 missing payload bytes
        assert!(parse_binary(&[0x02, b'k', 0x00, 0x01, 0x02]).is_err());
        // Unclosed dictionary
        assert!(parse_binary(&[0x00, b'k', 0x00, 0x02, b'x', 0x00, 1, 0, 0, 0]).is_err());
    }

    #[test]
    fn dict_helpers_mutate_in_place_without_disturbing_siblings() {
        let raw = build_sample_stats_bin();
        // Ensure mutated field stays ordered cleanly
        let mut doc = parse_binary(&raw).unwrap();
        assert!(doc.root.get_ci("CACHE").is_some());
        {
            let cache = doc.root.get_or_insert_dict_mut("cache");
            cache.set("PendingChanges", KvValue::Int32(0));
            let b0 = cache.get_dict_mut("0").unwrap();
            b0.get_or_insert_dict_mut("AchievementTimes").remove("0");
        }
        let encoded = serialize_binary(&doc);
        let reparsed = parse_binary(&encoded).unwrap();
        let cache = reparsed.root.get_dict("cache").unwrap();
        assert_eq!(cache.get("PendingChanges"), Some(&KvValue::Int32(0)));
        assert_eq!(
            cache
                .get_dict("0")
                .unwrap()
                .get_dict("AchievementTimes")
                .unwrap()
                .get("0"),
            None
        );
        assert_eq!(
            cache.get_dict("7").unwrap().get("data"),
            Some(&KvValue::Float32(12.5))
        );
    }
}
