//! Hubcap key status: the one classification both the startup check and the
//! command/status surfaces agree on.
//!
//! The provider does not expose an expiry date. The only authoritative signals
//! are HTTP 401/403 and `can_make_requests == false` on `/user/stats`; a
//! transport failure or a non-auth HTTP error stays `unknown` so a temporary
//! outage can never be reported as an expired key.

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use super::auth::UserStatsError;
use super::client::HubcapClient;
use super::types::HubcapUserStats;

/// Push channel for the saved-key verdict. It is emitted only when the
/// definitive state (or the key itself) changes; `unknown` never reaches the
/// UI, so a network blip cannot dismiss or duplicate a warning.
pub const HUBCAP_KEY_STATUS_EVENT: &str = "hubcap://key-status";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HubcapKeyState {
    Active,
    Inactive,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HubcapKeyReason {
    Unauthorized,
    AccountInactive,
    Unreachable,
    ProviderError,
}

impl HubcapKeyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::AccountInactive => "account_inactive",
            Self::Unreachable => "unreachable",
            Self::ProviderError => "provider_error",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HubcapKeyStatus {
    pub state: HubcapKeyState,
    pub reason: Option<HubcapKeyReason>,
    pub usage: Option<u32>,
    pub limit: Option<u32>,
}

impl HubcapKeyStatus {
    pub fn new(state: HubcapKeyState, reason: Option<HubcapKeyReason>) -> Self {
        Self {
            state,
            reason,
            usage: None,
            limit: None,
        }
    }

    pub fn with_usage(mut self, stats: &HubcapUserStats) -> Self {
        self.usage = stats.daily_usage;
        self.limit = stats.role_daily_limit.or(stats.daily_limit);
        self
    }

    /// `Some(true)` / `Some(false)` for a definitive answer, `None` when the
    /// provider could not be asked. The UI uses `None` to keep a previous
    /// warning instead of inventing a healthy state.
    pub fn active(&self) -> Option<bool> {
        match self.state {
            HubcapKeyState::Active => Some(true),
            HubcapKeyState::Inactive => Some(false),
            HubcapKeyState::Unknown => None,
        }
    }

    pub fn reason_str(&self) -> Option<&'static str> {
        self.reason.map(HubcapKeyReason::as_str)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubcapKeyStatusEvent {
    pub state: HubcapKeyState,
    pub reason: Option<HubcapKeyReason>,
    /// Short hex digest of the key the verdict belongs to. The UI drops an
    /// event whose fingerprint is not the key it currently shows, so a request
    /// still in flight for the previous key cannot raise or clear the warning
    /// for the new one.
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
struct StoredStatus {
    fingerprint: Vec<u8>,
    state: HubcapKeyState,
    reason: Option<HubcapKeyReason>,
}

fn status_slot() -> &'static Mutex<Option<StoredStatus>> {
    static SLOT: OnceLock<Mutex<Option<StoredStatus>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn status_handle() -> &'static OnceLock<AppHandle> {
    static HANDLE: OnceLock<AppHandle> = OnceLock::new();
    &HANDLE
}

/// Registers the push channel. Called once from `main.rs` before any command
/// can run; absence in unit tests simply skips the emit.
pub fn init(app: AppHandle) {
    let _ = status_handle().set(app);
}

fn key_fingerprint(api_key: &str) -> Vec<u8> {
    Sha256::digest(api_key.trim().as_bytes()).to_vec()
}

/// First 8 bytes of the SHA-256 digest, hex. Enough to tell two saved keys
/// apart without carrying (or exposing) the key itself.
pub fn key_fingerprint_hex(api_key: &str) -> String {
    key_fingerprint(api_key)
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Stores a definitive state and emits it when it differs from the previous
/// answer for the same key. A different key resets the comparison, so a new
/// expired key warns again even if the previous one did. Returns whether a
/// change was stored.
pub fn record(api_key: &str, state: HubcapKeyState, reason: Option<HubcapKeyReason>) -> bool {
    // An unknown answer must never overwrite a definitive warning: a temporary
    // outage is not evidence that the key became valid again.
    if state == HubcapKeyState::Unknown {
        return false;
    }

    let fingerprint = key_fingerprint(api_key);
    {
        let mut slot = status_slot()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let key_changed = slot
            .as_ref()
            .map(|status| status.fingerprint != fingerprint)
            .unwrap_or(true);
        let state_changed = slot
            .as_ref()
            .map(|status| status.state != state || status.reason != reason)
            .unwrap_or(true);
        if !key_changed && !state_changed {
            return false;
        }
        *slot = Some(StoredStatus {
            fingerprint,
            state,
            reason,
        });
    }

    if let Some(app) = status_handle().get() {
        let _ = app.emit(
            HUBCAP_KEY_STATUS_EVENT,
            HubcapKeyStatusEvent {
                state,
                reason,
                fingerprint: key_fingerprint_hex(api_key),
            },
        );
    }
    true
}

/// Drops the stored verdict. Used by "Clear AetherDesk caches"; the next
/// startup/usage check seeds it again.
pub fn clear() -> bool {
    let mut slot = status_slot()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let had_status = slot.is_some();
    *slot = None;
    had_status
}

/// The stored verdict for one key, if any. Test-only: production code learns
/// the state from the answer of `check_key_status` or from the emitted event.
#[cfg(test)]
pub fn snapshot(api_key: &str) -> Option<HubcapKeyStatusEvent> {
    let fingerprint = key_fingerprint(api_key);
    let slot = status_slot()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    slot.as_ref()
        .filter(|status| status.fingerprint == fingerprint)
        .map(|status| HubcapKeyStatusEvent {
            state: status.state,
            reason: status.reason,
            fingerprint: key_fingerprint_hex(api_key),
        })
}

/// HTTP-only classification. `status` is the final response status after
/// retries, `can_make_requests` is the `/user/stats` field when present.
pub(crate) fn classify_response(
    status: u16,
    can_make_requests: Option<bool>,
) -> (HubcapKeyState, Option<HubcapKeyReason>) {
    if status == 401 || status == 403 {
        return (
            HubcapKeyState::Inactive,
            Some(HubcapKeyReason::Unauthorized),
        );
    }
    if (200..300).contains(&status) {
        if can_make_requests == Some(false) {
            return (
                HubcapKeyState::Inactive,
                Some(HubcapKeyReason::AccountInactive),
            );
        }
        return (HubcapKeyState::Active, None);
    }
    (
        HubcapKeyState::Unknown,
        Some(HubcapKeyReason::ProviderError),
    )
}

impl HubcapClient {
    /// Performs the full `/user/stats` check and records the resulting state
    /// for the UI. This is the startup check: it is not cached by the
    /// five-minute validation cache, so opening the app always asks the
    /// provider once.
    pub async fn check_key_status(&self) -> HubcapKeyStatus {
        match self.fetch_user_stats("Key status check").await {
            Ok(stats) => {
                let (state, reason) = classify_response(200, stats.can_make_requests);
                let status = HubcapKeyStatus::new(state, reason).with_usage(&stats);
                self.record_state(&status);
                status
            }
            Err(UserStatsError::Unauthorized) => {
                let status = HubcapKeyStatus::new(
                    HubcapKeyState::Inactive,
                    Some(HubcapKeyReason::Unauthorized),
                );
                self.record_state(&status);
                status
            }
            Err(error) => {
                let status = HubcapKeyStatus::new(HubcapKeyState::Unknown, Some(error.reason()));
                self.record_state(&status);
                status
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_response, clear, key_fingerprint_hex, record, snapshot, HubcapKeyReason,
        HubcapKeyState,
    };

    #[test]
    fn an_authenticated_account_is_active_even_when_the_flag_is_absent() {
        assert_eq!(classify_response(200, None), (HubcapKeyState::Active, None));
        assert_eq!(
            classify_response(200, Some(true)),
            (HubcapKeyState::Active, None)
        );
    }

    #[test]
    fn an_account_that_cannot_make_requests_is_inactive() {
        assert_eq!(
            classify_response(200, Some(false)),
            (
                HubcapKeyState::Inactive,
                Some(HubcapKeyReason::AccountInactive)
            )
        );
    }

    #[test]
    fn rejected_credentials_are_inactive() {
        for status in [401, 403] {
            assert_eq!(
                classify_response(status, None),
                (
                    HubcapKeyState::Inactive,
                    Some(HubcapKeyReason::Unauthorized)
                )
            );
        }
    }

    #[test]
    fn rate_limits_and_server_errors_stay_unknown() {
        for status in [408, 425, 429, 500, 503] {
            assert_eq!(
                classify_response(status, None),
                (
                    HubcapKeyState::Unknown,
                    Some(HubcapKeyReason::ProviderError)
                )
            );
        }
    }

    #[test]
    fn definitive_states_are_deduplicated_and_reset_by_a_new_key() {
        clear();
        assert!(record(
            "key-a",
            HubcapKeyState::Inactive,
            Some(HubcapKeyReason::Unauthorized)
        ));
        assert!(!record(
            "key-a",
            HubcapKeyState::Inactive,
            Some(HubcapKeyReason::Unauthorized)
        ));
        assert_eq!(
            snapshot("key-a").map(|status| status.state),
            Some(HubcapKeyState::Inactive)
        );

        // A transport blip must not erase the warning.
        assert!(!record(
            "key-a",
            HubcapKeyState::Unknown,
            Some(HubcapKeyReason::Unreachable)
        ));
        assert_eq!(
            snapshot("key-a").map(|status| status.state),
            Some(HubcapKeyState::Inactive)
        );

        // A different key resets the comparison.
        assert!(record("key-b", HubcapKeyState::Active, None));
        assert_eq!(
            snapshot("key-b").map(|status| status.state),
            Some(HubcapKeyState::Active)
        );
        assert!(snapshot("key-a").is_none());
        clear();
    }

    #[test]
    fn the_fingerprint_is_stable_per_key_and_distinct_between_keys() {
        assert_eq!(
            key_fingerprint_hex("smm_alpha"),
            key_fingerprint_hex(" smm_alpha ")
        );
        assert_ne!(
            key_fingerprint_hex("smm_alpha"),
            key_fingerprint_hex("smm_beta")
        );
        assert_eq!(key_fingerprint_hex("smm_alpha").len(), 16);
    }
}
