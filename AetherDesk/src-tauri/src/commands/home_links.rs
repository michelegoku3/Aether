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
        encode_query_value(game_name)
    )
}

fn build_gcw_url(game_name: &str) -> String {
    format!(
        "https://gamecopyworld.com/games/pc_{}.shtml",
        build_gcw_slug(game_name)
    )
}

fn build_csrinru_url(game_name: &str) -> String {
    // CS.RIN.RU is a phpBB forum: its search endpoint supports the searched text in the
    // `keywords` query parameter, so we can deep-link to a pre-filled topic-title search.
    format!(
        "https://cs.rin.ru/forum/search.php?keywords={}&terms=all&sf=titleonly&sr=topics",
        encode_query_value(game_name)
    )
}

fn encode_query_value(value: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("q", value)
        .finish()
        .trim_start_matches("q=")
        .to_string()
}

fn build_gcw_slug(game_name: &str) -> String {
    game_name
        .to_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(normalize_gcw_token)
        .collect::<Vec<_>>()
        .join("_")
}

fn normalize_gcw_token(token: &str) -> String {
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
