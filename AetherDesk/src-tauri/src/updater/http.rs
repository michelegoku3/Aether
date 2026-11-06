//! The one HTTP client the self-updater uses.
//!
//! Every other network call in AetherDesk goes through
//! `providers::http::shared_client`, which always sets a timeout. The updater
//! did not: `reqwest::Client::new()` has no connect timeout and no read timeout,
//! so a connection that stalls before the headers or halfway through the body
//! leaves the update pending forever — the window shows "Preparing AetherDesk
//! portable update..." and never moves.
//!
//! There is deliberately no total-request timeout: the payload is the whole
//! portable ZIP (~18 MB), and a healthy download at 200 KB/s takes about 90
//! seconds. What must be bounded is silence, not duration.

use std::time::Duration;

use reqwest::Client;

/// Long enough for a slow DNS + TLS handshake, short enough to fail visibly.
const UPDATE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest tolerated silence between two body chunks.
const UPDATE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Client for release lookups and the ZIP download.
pub fn update_client() -> Client {
    update_client_with(UPDATE_CONNECT_TIMEOUT, UPDATE_IDLE_TIMEOUT)
}

/// Same client with explicit budgets, so a test can use sub-second values.
pub fn update_client_with(connect: Duration, idle: Duration) -> Client {
    Client::builder()
        .connect_timeout(connect)
        .read_timeout(idle)
        .user_agent("AetherDesk-Updater")
        .build()
        .expect("the update HTTP client must build")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_client_bounds_silence_and_connect() {
        // A builder cannot be introspected, so this pins the intent: both
        // budgets exist and the client builds. The behaviour behind them is
        // covered by the stalled/slow body tests in `tests/github_updater_tests`.
        let _ = update_client();
        assert!(UPDATE_CONNECT_TIMEOUT <= UPDATE_IDLE_TIMEOUT);
    }
}
