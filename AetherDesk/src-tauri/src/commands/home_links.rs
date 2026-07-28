use url::form_urlencoded;

#[tauri::command]
pub fn open_home_resource(site: String, game_name: String) -> Result<(), String> {
    let game_name = game_name.trim();
    if game_name.is_empty() {
        return Err("Select a Lua game before opening an external resource.".to_string());
    }

    let url = match site.as_str() {
        "onlinefix" => build_onlinefix_url(game_name),
        "gcw" => build_gcw_url(game_name),
        "csrinru" => build_csrinru_url(game_name),
        _ => return Err(format!("Unsupported external resource: {}", site)),
    };

    open_external_url(&url)
}

fn build_onlinefix_url(game_name: &str) -> String {
    format!(
        "https://online-fix.me/index.php?do=search&subaction=search&story={}",
        encode_query_value(&build_onlinefix_query(game_name))
    )
}

fn build_gcw_url(game_name: &str) -> String {
    format!(
        "https://gamecopyworld.com/games/pc_{}.shtml",
        build_gcw_slug(game_name)
    )
}

fn build_csrinru_url(game_name: &str) -> String {
    // CS.RIN.RU is phpBB. The most reliable deep-link is an advanced title-topic search
    // with a conservative, punctuation-free query. This avoids brittle per-game aliases and
    // handles titles such as "Baldur's Gate 3" (thread title uses III) and
    // "DAVIGO: VR vs. PC" (thread title starts with DAVIGO plus extra tags) better than
    // passing the raw Steam title verbatim.
    format!(
        "https://cs.rin.ru/forum/search.php?keywords={}&terms=all&author=&sc=1&sf=titleonly&sk=t&sd=d&sr=topics&st=0&ch=300&t=0&submit=Search",
        encode_query_value(&build_csrinru_query(game_name))
    )
}

fn build_onlinefix_query(game_name: &str) -> String {
    normalize_query_title(game_name, QueryFlavor::OnlineFix)
}

fn build_csrinru_query(game_name: &str) -> String {
    normalize_query_title(game_name, QueryFlavor::CsRinRu)
}

#[derive(Debug, Clone, Copy)]
enum QueryFlavor {
    OnlineFix,
    CsRinRu,
}

fn normalize_query_title(game_name: &str, flavor: QueryFlavor) -> String {
    let without_brackets = remove_bracketed_segments(game_name);
    let base_title = strip_subtitle_after_colon(&without_brackets);
    let without_editions = strip_known_edition_suffixes(&base_title);
    let possessive_policy = match flavor {
        QueryFlavor::OnlineFix => PossessivePolicy::KeepAsPlainS,
        QueryFlavor::CsRinRu => PossessivePolicy::DropPossessiveS,
    };

    let tokens = title_tokens(&without_editions, TokenOptions {
        preserve_hyphen: false,
        possessive_policy,
        roman_numerals: RomanNumeralPolicy::Keep,
        drop_numeric_tokens: matches!(flavor, QueryFlavor::CsRinRu),
    });

    tokens.join(" ")
}

fn encode_query_value(value: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("q", value)
        .finish()
        .trim_start_matches("q=")
        .to_string()
}

fn build_gcw_slug(game_name: &str) -> String {
    let without_brackets = remove_bracketed_segments(game_name);
    let without_editions = strip_known_edition_suffixes(&without_brackets);
    let tokens = title_tokens(&without_editions, TokenOptions {
        preserve_hyphen: true,
        possessive_policy: PossessivePolicy::KeepAsPlainS,
        roman_numerals: RomanNumeralPolicy::ConvertToArabic,
        drop_numeric_tokens: false,
    });

    tokens.join("_")
}

#[derive(Debug, Clone, Copy)]
struct TokenOptions {
    preserve_hyphen: bool,
    possessive_policy: PossessivePolicy,
    roman_numerals: RomanNumeralPolicy,
    drop_numeric_tokens: bool,
}

#[derive(Debug, Clone, Copy)]
enum PossessivePolicy {
    KeepAsPlainS,
    DropPossessiveS,
}

#[derive(Debug, Clone, Copy)]
enum RomanNumeralPolicy {
    Keep,
    ConvertToArabic,
}

fn title_tokens(title: &str, options: TokenOptions) -> Vec<String> {
    let normalized = normalize_apostrophes(title)
        .replace('™', "")
        .replace('®', "")
        .replace('©', "")
        // Dotted acronyms such as R.E.P.O. are indexed by most sites as "repo".
        .replace('.', "")
        .replace('&', " and ");

    let normalized = match options.possessive_policy {
        PossessivePolicy::KeepAsPlainS => normalized.replace("'", ""),
        PossessivePolicy::DropPossessiveS => remove_possessive_s(&normalized),
    };

    let mut cleaned = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        if ch.is_ascii_alphanumeric() || (options.preserve_hyphen && ch == '-') {
            cleaned.push(ch.to_ascii_lowercase());
        } else {
            cleaned.push(' ');
        }
    }

    cleaned
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .filter(|token| !options.drop_numeric_tokens || !token.chars().all(|ch| ch.is_ascii_digit()))
        .map(|token| match options.roman_numerals {
            RomanNumeralPolicy::Keep => token.to_string(),
            RomanNumeralPolicy::ConvertToArabic => normalize_roman_token(token),
        })
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

fn normalize_roman_token(token: &str) -> String {
    match token {
        "i" => "1",
        "ii" => "2",
        "iii" => "3",
        "iv" => "4",
        "v" => "5",
        "vi" => "6",
        "vii" => "7",
        "viii" => "8",
        "ix" => "9",
        "x" => "10",
        _ => token,
    }
    .to_string()
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

fn strip_subtitle_after_colon(value: &str) -> String {
    value
        .split_once(':')
        .map(|(before, _)| before.trim().to_string())
        .filter(|before| !before.is_empty())
        .unwrap_or_else(|| value.trim().to_string())
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

#[cfg(target_os = "windows")]
fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open the external resource in the default browser: {}", e))
}

#[cfg(target_os = "macos")]
fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open the external resource in the default browser: {}", e))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open the external resource in the default browser: {}", e))
}
