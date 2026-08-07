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
pub const MAX_VARIANTS: usize = 4;

/// Common franchise abbreviations users type instead of full names.
/// Expansions are alternatives — any of them OR the original token may hit.
/// Keys must be lowercase single tokens.
static ALIAS_EXPANSIONS: &[(&str, &[&str])] = &[
    ("ac", &["assassins creed", "assassin s creed"]),
    ("acnh", &["animal crossing new horizons"]),
    ("aoe", &["age of empires"]),
    ("aoe2", &["age of empires 2", "age of empires ii"]),
    ("aoe4", &["age of empires 4", "age of empires iv"]),
    ("apex", &["apex legends"]),
    ("ats", &["american truck simulator"]),
    ("bf", &["battlefield"]),
    ("bf1", &["battlefield 1"]),
    ("bf2042", &["battlefield 2042"]),
    ("bf4", &["battlefield 4"]),
    ("bfv", &["battlefield v"]),
    ("bl2", &["borderlands 2"]),
    ("bl3", &["borderlands 3"]),
    ("botw", &["the legend of zelda breath of the wild", "breath of the wild"]),
    ("btd", &["bloons td"]),
    ("civ", &["civilization"]),
    ("ck3", &["crusader kings 3"]),
    ("cod", &["call of duty"]),
    ("cp", &["cyberpunk", "cyberpunk 2077"]),
    ("cp2077", &["cyberpunk 2077"]),
    ("cs", &["counter strike", "counter-strike"]),
    ("cs2", &["counter strike 2", "counter-strike 2"]),
    ("csgo", &["counter strike global offensive", "counter-strike global offensive"]),
    ("css", &["counter strike source", "counter-strike source"]),
    ("d2", &["diablo 2", "diablo ii", "destiny 2"]),
    ("d3", &["diablo 3", "diablo iii"]),
    ("d4", &["diablo 4", "diablo iv"]),
    ("dbd", &["dead by daylight"]),
    ("dota", &["dota 2"]),
    ("dota2", &["dota 2"]),
    ("ds", &["dark souls"]),
    ("ds1", &["dark souls"]),
    ("ds2", &["dark souls 2", "dark souls ii"]),
    ("ds3", &["dark souls 3", "dark souls iii"]),
    ("eft", &["escape from tarkov"]),
    ("er", &["elden ring"]),
    ("eso", &["the elder scrolls online"]),
    ("ets2", &["euro truck simulator 2"]),
    ("eu4", &["europa universalis 4"]),
    ("fc", &["far cry"]),
    ("fc5", &["far cry 5"]),
    ("fc6", &["far cry 6"]),
    ("ff", &["final fantasy"]),
    ("fh", &["forza horizon"]),
    ("fh4", &["forza horizon 4"]),
    ("fh5", &["forza horizon 5"]),
    ("fm", &["forza motorsport"]),
    ("fn", &["fortnite"]),
    ("fnaf", &["five nights at freddy", "five nights at freddy's"]),
    ("fo4", &["fallout 4"]),
    ("fo76", &["fallout 76"]),
    ("fonv", &["fallout new vegas"]),
    ("got", &["ghost of tsushima"]),
    ("gow", &["god of war"]),
    ("gt", &["gran turismo"]),
    ("gt7", &["gran turismo 7"]),
    ("gta", &["grand theft auto"]),
    ("gta5", &["grand theft auto 5", "grand theft auto v"]),
    ("gta6", &["grand theft auto 6", "grand theft auto vi"]),
    ("hfw", &["horizon forbidden west"]),
    ("hk", &["hollow knight"]),
    ("hl", &["half life"]),
    ("hl2", &["half life 2"]),
    ("hoi4", &["hearts of iron 4"]),
    ("hots", &["heroes of the storm"]),
    ("hzd", &["horizon zero dawn"]),
    ("isaac", &["the binding of isaac"]),
    ("kh", &["kingdom hearts"]),
    ("l4d", &["left 4 dead"]),
    ("l4d2", &["left 4 dead 2"]),
    ("lol", &["league of legends"]),
    ("mc", &["minecraft"]),
    ("mh", &["monster hunter"]),
    ("mk", &["mortal kombat"]),
    ("nba", &["nba 2k", "nba2k"]),
    ("nfs", &["need for speed"]),
    ("ow", &["overwatch"]),
    ("ow2", &["overwatch 2"]),
    ("p3", &["persona 3"]),
    ("p4", &["persona 4"]),
    ("p5", &["persona 5"]),
    ("poe", &["path of exile"]),
    ("poe2", &["path of exile 2"]),
    ("pubg", &["playerunknown s battlegrounds", "playerunknowns battlegrounds"]),
    ("r6", &["rainbow six siege", "rainbow 6 siege"]),
    ("r6s", &["rainbow six siege", "rainbow 6 siege"]),
    ("rdr", &["red dead redemption"]),
    ("rdr2", &["red dead redemption 2"]),
    ("re", &["resident evil"]),
    ("tf2", &["team fortress 2"]),
    ("rl", &["rocket league"]),
    ("sc2", &["starcraft 2", "starcraft ii"]),
    ("sdv", &["stardew valley"]),
    ("sf", &["street fighter"]),
    ("skyrim", &["the elder scrolls v skyrim", "skyrim"]),
    ("tboi", &["the binding of isaac"]),
    ("tes5", &["the elder scrolls v skyrim", "skyrim"]),
    ("tk", &["tekken"]),
    ("tlou", &["the last of us"]),
    ("tlou2", &["the last of us part 2", "the last of us part ii"]),
    ("totk", &["the legend of zelda tears of the kingdom", "tears of the kingdom"]),
    ("tw", &["total war"]),
    ("tw3", &["the witcher 3", "witcher 3"]),
    ("val", &["valorant"]),
    ("wh", &["warhammer"]),
    ("witcher", &["the witcher"]),
    ("wow", &["world of warcraft"]),
    ("wukong", &["black myth wukong"]),
    ("zelda", &["the legend of zelda"]),
];



fn lookup(token: &str) -> Option<&'static [&'static str]> {
    ALIAS_EXPANSIONS
        .iter()
        .find(|(key, _)| *key == token)
        .map(|(_, expansions)| *expansions)
}

/// Strip punctuation/symbols from a token so `GTA:` → `gta` still hits the alias map.
/// Keeps only alphanumeric lowercased.
fn clean_token(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn lookup_with_fallback(token: &str) -> Option<&'static [&'static str]> {
    let lower = token.to_lowercase();
    if let Some(v) = lookup(&lower) {
        return Some(v);
    }
    let cleaned = clean_token(token);
    if cleaned != lower {
        if let Some(v) = lookup(&cleaned) {
            return Some(v);
        }
    }
    None
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

    let push_unique = |candidate: String, out: &mut Vec<String>, seen: &mut Vec<String>| {
        let key = candidate.to_lowercase();
        if !seen.contains(&key) && out.len() < MAX_VARIANTS {
            seen.push(key);
            out.push(candidate);
        }
    };

    // Whole-query alias hit ("gta" alone, "wukong" alone, ...).
    // Try both raw lower and cleaned form so "GTA:" still expands.
    let raw_lower = raw.to_lowercase();
    let mut whole_hit_expansions: Option<&[&str]> = lookup(&raw_lower);
    if whole_hit_expansions.is_none() {
        let cleaned_whole = clean_token(&raw);
        if cleaned_whole != raw_lower {
            whole_hit_expansions = lookup(&cleaned_whole);
        }
    }
    if let Some(expansions) = whole_hit_expansions {
        for expansion in expansions {
            push_unique(expansion.to_string(), &mut out, &mut seen);
        }
    }

    // Per-token swap: for each token that has an alias, build a new query with
    // that token replaced, leaving the other tokens untouched.
    // Token lookup is punctuation-insensitive via clean_token (e.g. "GTA:" → "gta").
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        let Some(expansions) = lookup_with_fallback(token) else {
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
