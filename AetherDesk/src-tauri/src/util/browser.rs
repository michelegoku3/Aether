/// Open a URL in the system default browser.
///
/// Centralised here so both `commands/library.rs` (SteamDB links) and
/// `commands/home_links.rs` (OnlineFix, GCW, CSRINRU) share the same
/// cross-platform implementation.
#[cfg(target_os = "windows")]
pub fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
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
