//! Normalization & sanitization helpers (ported/adapted from SFF).
//!
//! - `normalize_string`  —  punctuation-insensitive, case-insensitive key for
//!   relevance scoring and fuzzy matching. Mirrors SFF's `_normalize_for_search`
//!   (non-alphanumeric → space, NFKD / symbol stripping, lowercasing, whitespace
//!   collapse) plus Aether's light Roman-numeral synonym pass (`ix`→`9` …).
//!   Pure, no I/O, trivially testable.
//!
//! - `sanitize_query_for_hubcap`  —  minimal, reversible clean for Hubcap query
//!   variants: replaces punctuation/symbols with a single space so
//!   `"Take Me To The Dungeon!!"` → `"Take Me To The Dungeon"`.
//!   Used to generate an extra Hubcap `search`/`library` variant when the raw
//!   user query contains decorative punctuation that Hubcap's substring matcher
//!   treats literally (observed 400 soft-fail on `!!` queries).

/// Normalize a string for high-fidelity matching.
///
/// 1. Lowercase.
/// 2. Any character that is not alphanumeric is treated as a separator (space).
///    This covers _all_ punctuation (`!`, `:`, `.`, `'`, `®`, `™`, `-`, `_`, …)
///    and the Unicode symbol categories that SFF drops via `Mn`/`S` + NFKD.
///    Using a single space keeps token boundaries (so `GTA:San` → `gta san`,
///    not `gtasan`).
/// 3. Collapse whitespace.
/// 4. Per-token Roman → digit synonym pass (`ix`→`9` …) — `civ`→`civilization` è ora un alias dedicato.
///
/// This replaces the former narrow replace-chain (`.`, `'`, `:`, `®`, `™`, `-`, `_`)
/// which left `!` etc untouched and broke exact/prefix checks for titles with
/// trailing `!!`.
///
/// Ranking helper for Hubcap-only tail rows. Steam hits are never dropped
/// by this score. No typo/Levenshtein: that cannot beat Steam's own search.
pub fn relevance_score(query: &str, name: &str) -> usize {
    let q_norm = normalize_string(query);
    let n_norm = normalize_string(name);

    if q_norm.is_empty() {
        return 10000;
    }
    if q_norm == n_norm {
        return 0;
    }
    if n_norm.starts_with(&q_norm) {
        return 1 + (n_norm.len() - q_norm.len());
    }
    if n_norm.contains(&q_norm) {
        let pos = n_norm.find(&q_norm).unwrap_or(0);
        return 100 + pos + (n_norm.len() - q_norm.len());
    }
    10000
}

pub fn normalize_string(s: &str) -> String {
    let lower = s.to_lowercase();

    // 1 → space for any non-alphanumeric separator.
    // `is_alphanumeric` is Unicode-aware (keeps letters with accents as alnum;
    // SFF would NFKD-strip them to ASCII — close enough for game titles).
    let mut spaced = String::with_capacity(lower.len());
    for ch in lower.chars() {
        if ch.is_alphanumeric() {
            spaced.push(ch);
        } else if ch.is_whitespace() {
            spaced.push(' ');
        } else {
            // punctuation / symbol / trademark / etc → space
            spaced.push(' ');
        }
    }

    // Collapse + Roman pass (civ -> civilization is now an alias, not a roman numeral,
    // so that "Civ 6" expands via the alias table consistently with other franchises).
    let words: Vec<String> = spaced
        .split_whitespace()
        .map(|w| match w {
            "ix" => "9".to_string(),
            "viii" => "8".to_string(),
            "vii" => "7".to_string(),
            "vi" => "6".to_string(),
            "v" => "5".to_string(),
            "iv" => "4".to_string(),
            "iii" => "3".to_string(),
            "ii" => "2".to_string(),
            "i" => "1".to_string(),
            _ => w.to_string(),
        })
        .collect();

    words.join(" ")
}

/// Minimal sanitizer for Hubcap query variants.
///
/// Returns `Some(sanitized)` when the cleaned query differs from the trimmed
/// original (case-insensitive compare) and is non-empty, otherwise `None` so
/// callers can skip the extra network call.
/// Keeps the original casing (Hubcap is case-insensitive anyway) but replaces
/// punctuation/symbols with space and collapses whitespace.
pub fn sanitize_query_for_hubcap(query: &str) -> Option<String> {
    let raw_trimmed = query.trim();
    if raw_trimmed.is_empty() {
        return None;
    }
    let mut spaced = String::with_capacity(raw_trimmed.len());
    for ch in raw_trimmed.chars() {
        if ch.is_alphanumeric() || ch.is_whitespace() {
            spaced.push(ch);
        } else {
            spaced.push(' ');
        }
    }
    let collapsed = spaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if collapsed.is_empty() {
        return None;
    }
    // Deduplicate case-insensitively against the original.
    if collapsed.to_lowercase() == raw_trimmed.to_lowercase() {
        return None;
    }
    // Also deduplicate against lowercased normalized form collapsing already done;
    // if only case changed, not useful.
    if collapsed.eq_ignore_ascii_case(raw_trimmed) {
        return None;
    }
    Some(collapsed)
}

/// Placeholder retained for backward compatibility; real Hubcap variant expansion
/// lives in `store::service::collect_hubcap_hits` to avoid a circular dependency
/// on `store::aliases`. Kept here so external callers/tests can keep importing
/// through `store::normalize`.
#[allow(dead_code)]
pub fn hubcap_query_variants(_query: &str, _max: usize) -> Vec<String> {
    Vec::new()
}
