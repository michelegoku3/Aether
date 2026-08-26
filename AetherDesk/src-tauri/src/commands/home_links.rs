use crate::util::browser::open_external_url;
use url::form_urlencoded;

#[tauri::command]
pub fn open_home_resource(site: String, game_name: String) -> Result<(), String> {
    let game_name = game_name.trim();
    if game_name.is_empty() {
        return Err("Select a Lua game before opening an external resource.".to_string());
    }

    let url = match site.as_str() {
        "ofme" => build_ofme_url(game_name),
        "gcw" => build_gcw_url(game_name),
        "csrinru" => build_csrinru_url(game_name),
        _ => {
            crate::desk_log_warn!("home_links", "Unsupported external resource site requested: '{}'", site);
            return Err(format!("Unsupported external resource: {}", site));
        }
    };

    crate::desk_log_info!("home_links", "Opening external resource site='{}' for game '{}' (url: {})", site, game_name, url);
    open_external_url(&url)
}

// OFME = online-fix.me (la crack). Il nome funzione evita "onlinefix", che
    // nelle AI si confonde con la modalità AetherOnline (il payload Aether).
    pub(crate) fn build_ofme_url(game_name: &str) -> String {
    format!(
        "https://online-fix.me/index.php?do=search&subaction=search&story={}",
        encode_query_value(&build_ofme_query(game_name))
    )
}

pub(crate) fn build_gcw_url(game_name: &str) -> String {
    // GCW page slugs are not mechanically derivable from Steam titles: many
    // older pages use historical series numbers or other hand-written names.
    // A generic button must therefore open GCW's own search page instead of
    // guessing a direct `pc_<slug>.shtml` path.
    format!(
        "https://gamecopyworld.eu/games/search_results.shtml?q={}",
        encode_query_value(&build_gcw_query(game_name))
    )
}

pub(crate) fn build_csrinru_url(game_name: &str) -> String {
    // CS.RIN.RU is phpBB. The most reliable deep-link is an advanced title-topic search
    // with a conservative, punctuation-free query. This avoids brittle per-game aliases and
    // handles titles such as "Baldur's Gate 3" (thread title uses III) and
    // "DAVIGO: VR vs. PC" (thread title starts with DAVIGO plus extra tags) better than
    // passing the raw Steam title verbatim.
    format!(
        "https://cs.rin.ru/forum/search.php?keywords={}&fid%5B%5D=10&terms=all&author=&sc=1&sf=titleonly&sk=t&sd=d&sr=topics&st=0&ch=300&t=0&submit=Search",
        encode_query_value(&build_csrinru_query(game_name))
    )
}

fn build_ofme_query(game_name: &str) -> String {
    normalize_query_title(game_name, QueryFlavor::Ofme)
}

fn build_csrinru_query(game_name: &str) -> String {
    normalize_query_title(game_name, QueryFlavor::CsRinRu)
}

fn build_gcw_query(game_name: &str) -> String {
    let without_brackets = remove_bracketed_segments(game_name);
    let without_editions = strip_known_edition_suffixes(&without_brackets);
    title_tokens(
        &without_editions,
        TokenOptions {
            preserve_hyphen: false,
            preserve_dot: false,
            possessive_policy: PossessivePolicy::KeepAsPlainS,
            // GCW search is text search. Keep roman numerals as written because
            // page titles often use them (`Black Ops II`, `DARK SOULS III`).
            drop_numeric_tokens: false,
        },
    )
    .join(" ")
}

#[derive(Debug, Clone, Copy)]
enum QueryFlavor {
    Ofme,
    CsRinRu,
}

fn normalize_query_title(game_name: &str, flavor: QueryFlavor) -> String {
    let without_brackets = remove_bracketed_segments(game_name);
    let base_title = match flavor {
        // OFME (online-fix.me) searches should keep real subtitles after ':' so titles like
        // "Call of Duty: Black Ops II" do not degrade to plain "Call of Duty".
        QueryFlavor::Ofme => without_brackets.trim().to_string(),
        // CSRINRU needs the Davigo-style cleanup, but only when the subtitle is
        // platform/mode noise such as "VR vs. PC". Meaningful subtitles like
        // "Shadows Die Twice" are kept.
        QueryFlavor::CsRinRu => strip_noisy_colon_subtitle(&without_brackets),
    };
    let without_editions = strip_known_edition_suffixes(&base_title);
    let possessive_policy = match flavor {
        QueryFlavor::Ofme => PossessivePolicy::KeepAsPlainS,
        QueryFlavor::CsRinRu => PossessivePolicy::DropPossessiveS,
    };

    let tokens = title_tokens(
        &without_editions,
        TokenOptions {
            preserve_hyphen: false,
            preserve_dot: matches!(flavor, QueryFlavor::CsRinRu),
            possessive_policy,
            drop_numeric_tokens: matches!(flavor, QueryFlavor::CsRinRu),
        },
    );

    tokens.join(" ")
}

fn encode_query_value(value: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("q", value)
        .finish()
        .trim_start_matches("q=")
        .to_string()
}

#[derive(Debug, Clone, Copy)]
struct TokenOptions {
    preserve_hyphen: bool,
    preserve_dot: bool,
    possessive_policy: PossessivePolicy,
    drop_numeric_tokens: bool,
}

#[derive(Debug, Clone, Copy)]
enum PossessivePolicy {
    KeepAsPlainS,
    DropPossessiveS,
}

fn title_tokens(title: &str, options: TokenOptions) -> Vec<String> {
    let normalized = normalize_apostrophes(title)
        .replace('™', "")
        .replace('®', "")
        .replace('©', "")
        .replace('&', " and ");

    let normalized = if options.preserve_dot {
        normalized
    } else {
        // Dotted acronyms such as R.E.P.O. are indexed by OFME/GCW as "repo".
        normalized.replace('.', "")
    };

    let normalized = match options.possessive_policy {
        PossessivePolicy::KeepAsPlainS => normalized.replace("'", ""),
        PossessivePolicy::DropPossessiveS => remove_possessive_s(&normalized),
    };

    let mut cleaned = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        if ch.is_ascii_alphanumeric()
            || (options.preserve_hyphen && ch == '-')
            || (options.preserve_dot && ch == '.')
        {
            cleaned.push(ch.to_ascii_lowercase());
        } else {
            cleaned.push(' ');
        }
    }

    cleaned
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .filter(|token| {
            !options.drop_numeric_tokens || !token.chars().all(|ch| ch.is_ascii_digit())
        })
        .map(|token| token.to_string())
        .collect()
}

fn normalize_apostrophes(value: &str) -> String {
    value
        .replace('’', "'")
        .replace('‘', "'")
        .replace('`', "'")
        .replace('´', "'")
}

fn remove_possessive_s(value: &str) -> String {
    value
        .replace("'s", " ")
        .replace("'S", " ")
        .replace('’', "'")
}

fn remove_bracketed_segments(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut depth = 0usize;

    for ch in value.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => output.push(ch),
            _ => {}
        }
    }

    output.trim().to_string()
}

fn strip_noisy_colon_subtitle(value: &str) -> String {
    let Some((before, after)) = value.split_once(':') else {
        return value.trim().to_string();
    };

    let before = before.trim();
    if before.is_empty() {
        return value.trim().to_string();
    }

    let after_tokens = basic_ascii_tokens(after);
    if after_tokens.is_empty() {
        return value.trim().to_string();
    }

    let has_platform_noise = after_tokens
        .iter()
        .any(|token| PLATFORM_NOISE_TOKENS.contains(&token.as_str()));
    let only_noise_or_connectors = after_tokens.iter().all(|token| {
        PLATFORM_NOISE_TOKENS.contains(&token.as_str())
            || CONNECTOR_TOKENS.contains(&token.as_str())
    });

    if has_platform_noise && only_noise_or_connectors {
        before.to_string()
    } else {
        value.trim().to_string()
    }
}

fn basic_ascii_tokens(value: &str) -> Vec<String> {
    title_tokens(
        value,
        TokenOptions {
            preserve_hyphen: false,
            preserve_dot: false,
            possessive_policy: PossessivePolicy::KeepAsPlainS,
            drop_numeric_tokens: false,
        },
    )
}

fn strip_known_edition_suffixes(value: &str) -> String {
    let mut current = value.trim().to_string();

    loop {
        let lower = current.to_lowercase();
        let Some(suffix) = KNOWN_EDITION_SUFFIXES
            .iter()
            .find(|suffix| lower.ends_with(**suffix))
        else {
            break;
        };

        let new_len = current.len().saturating_sub(suffix.len());
        current.truncate(new_len);
        current = current
            .trim_matches(|ch| matches!(ch, ' ' | '-' | ':' | '–' | '—'))
            .trim()
            .to_string();
    }

    if current.is_empty() {
        value.trim().to_string()
    } else {
        current
    }
}

const PLATFORM_NOISE_TOKENS: &[&str] = &[
    "vr",
    "pc",
    "optional",
    "vive",
    "rift",
    "oculus",
    "quest",
    "index",
    "psvr",
    "psvr2",
    "steamvr",
    "windows",
    "linux",
    "mac",
    "macos",
    "mode",
    "modes",
];

const CONNECTOR_TOKENS: &[&str] = &["vs", "versus", "and", "or", "with", "for"];

const KNOWN_EDITION_SUFFIXES: &[&str] = &[
    " game of the year enhanced",
    " game of the year edition",
    " game of the year",
    " goty enhanced",
    " goty edition",
    " goty",
    " enhanced edition",
    " definitive edition",
    " digital deluxe edition",
    " deluxe edition",
    " ultimate edition",
    " complete edition",
    " collector's edition",
    " collectors edition",
    " standard edition",
];
