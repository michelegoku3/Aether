/// Open a URL in the system default browser.
///
/// Centralised here so both `commands/library.rs` (SteamDB links) and
/// `commands/home_links.rs` (OnlineFix, GCW, CSRINRU) share the same
/// cross-platform implementation.
#[cfg(target_os = "windows")]
pub fn open_external_url(url: &str) -> Result<(), String> {
    // Do NOT route URLs through `cmd /C start`: `&` is a command separator in
    // cmd.exe, and search URLs such as `...?do=search&subaction=search&story=...`
    // get truncated to the first parameter. That is exactly what makes
    // OnlineFix open a blank search and CSRINRU ignore `sf=titleonly&sr=topics`.
    // `rundll32 url.dll,FileProtocolHandler` invokes the shell URL handler
    // directly, preserving the full query string.
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open URL in default browser: {}", e))
}

#[cfg(target_os = "macos")]
pub fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open URL in default browser: {}", e))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open URL in default browser: {}", e))
}
