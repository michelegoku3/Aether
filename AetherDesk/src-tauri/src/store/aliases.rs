//! Query alias expansion for remote search backends with plain substring matching.
//!
//! Hubcap's `/library` and `/search` endpoints do not know about abbreviations:
//! a user typing `gta san andreas` would never hit a title stored as
//! "Grand Theft Auto: San Andreas". For each known alias token we generate extra
//! query variants with the token swapped for its full expansion.
//!
//! This module is pure (no I/O, no state) so it stays trivially testable and can
//! be reused by any provider that needs it. Ported from SFF's
//! `_alias_expanded_queries` / `_ALIAS_EXPANSIONS`.

/// Maximum number of variants returned, original query included.
/// Caps the fan-out so a query with two aliased tokens does not explode
/// into N*M remote requests (mirrors SFF's cap of 6, kept tighter here
/// because every variant costs real network calls).
const MAX_VARIANTS: usize = 4;

/// Common franchise abbreviations users type instead of full names.
/// Expansions are alternatives — any of them OR the original token may hit.
/// Keys must be lowercase single tokens.
static ALIAS_EXPANSIONS: &[(&str, &[&str])] = &[
    ("gta", &["grand theft auto"]),
    ("rdr", &["red dead redemption"]),
    ("cod", &["call of duty"]),
    ("re", &["resident evil"]),
    ("tf2", &["team fortress 2"]),
    ("csgo", &["counter strike global offensive", "counter-strike global offensive"]),
    ("cs2", &["counter strike 2", "counter-strike 2"]),
    ("css", &["counter strike source", "counter-strike source"]),
    ("cs", &["counter strike", "counter-strike"]),
    ("kh", &["kingdom hearts"]),
    ("mh", &["monster hunter"]),
    ("ff", &["final fantasy"]),
    ("ds", &["dark souls"]),
    ("ds2", &["dark souls 2", "dark souls ii"]),
    ("ds3", &["dark souls 3", "dark souls iii"]),
    ("er", &["elden ring"]),
    ("mk", &["mortal kombat"]),
    ("ac", &["assassins creed", "assassin s creed"]),
    ("btd", &["bloons td"]),
    ("tw", &["total war"]),
    ("wh", &["warhammer"]),
    ("sf", &["street fighter"]),
    ("tk", &["tekken"]),
    ("p5", &["persona 5"]),
    ("p4", &["persona 4"]),
    ("p3", &["persona 3"]),
    ("pubg", &["playerunknown s battlegrounds", "playerunknowns battlegrounds"]),
    ("wukong", &["black myth wukong"]),
    ("nba", &["nba 2k", "nba2k"]),
];

fn lookup(token: &str) -> Option<&'static [&'static str]> {
    ALIAS_EXPANSIONS
        .iter()
        .find(|(key, _)| *key == token)
        .map(|(_, expansions)| *expansions)
}

/// Yield candidate query strings for remote search backends.
///
/// The original query is always first; expansions follow. Duplicates are
/// de-duped case-insensitively and the result is capped at [`MAX_VARIANTS`].
/// Empty/blank input yields an empty vector.
pub fn expanded_queries(query: &str) -> Vec<String> {
    let raw = query.trim();
    if raw.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<String> = vec![raw.to_string()];
    let mut seen: Vec<String> = vec![raw.to_lowercase()];

    let mut push_unique = |candidate: String, out: &mut Vec<String>, seen: &mut Vec<String>| {
        let key = candidate.to_lowercase();
        if !seen.contains(&key) && out.len() < MAX_VARIANTS {
            seen.push(key);
            out.push(candidate);
        }
    };

    // Whole-query alias hit ("gta" alone, "wukong" alone, ...).
    if let Some(expansions) = lookup(&raw.to_lowercase()) {
        for expansion in expansions {
            push_unique(expansion.to_string(), &mut out, &mut seen);
        }
    }

    // Per-token swap: for each token that has an alias, build a new query with
    // that token replaced, leaving the other tokens untouched.
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        let Some(expansions) = lookup(&token.to_lowercase()) else {
            continue;
        };
        for expansion in expansions {
            let mut new_tokens: Vec<&str> = tokens.clone();
            new_tokens[index] = expansion;
            push_unique(new_tokens.join(" "), &mut out, &mut seen);
        }
    }

    out
}

/// Convenience helper: the original query plus at most the first alias
/// expansion. Used when the caller has a tight request budget and wants a
/// single extra shot at the remote backend (mirrors SFF's Hubcap merge, which
/// appends only the first alias variant).
pub fn primary_variants(query: &str) -> Vec<String> {
    expanded_queries(query).into_iter().take(2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_blank_query_yields_nothing() {
        assert!(expanded_queries("").is_empty());
        assert!(expanded_queries("   ").is_empty());
    }

    #[test]
    fn plain_query_has_only_the_original_variant() {
        assert_eq!(expanded_queries("cyberpunk 2077"), vec!["cyberpunk 2077"]);
    }

    #[test]
    fn whole_query_alias_expands() {
        let variants = expanded_queries("gta");
        assert_eq!(variants[0], "gta");
        assert!(variants.contains(&"grand theft auto".to_string()));
    }

    #[test]
    fn token_alias_expands_in_place() {
        let variants = expanded_queries("gta san andreas");
        assert!(variants.contains(&"grand theft auto san andreas".to_string()));
    }

    #[test]
    fn expansion_is_case_insensitive_and_deduped() {
        let variants = expanded_queries("GTA");
        assert_eq!(variants[0], "GTA");
        let lowercase: Vec<String> = variants.iter().map(|v| v.to_lowercase()).collect();
        let mut deduped = lowercase.clone();
        deduped.dedup();
        deduped.sort();
        let mut sorted = lowercase.clone();
        sorted.sort();
        assert_eq!(sorted, deduped);
    }

    #[test]
    fn fan_out_is_capped() {
        let variants = expanded_queries("cs go gta re cod");
        assert!(variants.len() <= MAX_VARIANTS);
    }

    #[test]
    fn primary_variants_returns_at_most_two() {
        assert!(primary_variants("csgo").len() <= 2);
        assert_eq!(primary_variants("stray"), vec!["stray"]);
    }
}
