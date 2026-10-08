//! Candidate selection for the provider-backed pin-refresh lane.

use super::status::now_epoch;
use crate::manifest::pins::LuaManifestPins;
use std::collections::HashMap;
use std::time::Duration;

const CONTENTS_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const CONTENTS_IDLE_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Selects installed, updates-enabled games whose contents check is due.
/// Steam-side changes use the fast cadence; unchanged games use the idle one.
pub(crate) fn pin_refresh_candidates(
    contents_checked: &HashMap<u32, u64>,
    contents_fingerprint: &HashMap<u32, String>,
    current_acf: &HashMap<u32, String>,
    current_luas: &HashMap<u32, String>,
    steam_path: &str,
) -> Vec<u32> {
    let now = now_epoch();
    let mut candidates = Vec::new();
    for app_id in current_luas.keys() {
        let Some(fingerprint) = current_acf.get(app_id) else {
            continue;
        };
        let last_check = contents_checked.get(app_id).copied().unwrap_or(0);
        let changed_since_check = contents_fingerprint.get(app_id) != Some(fingerprint);
        let interval = if changed_since_check {
            CONTENTS_REFRESH_INTERVAL
        } else {
            CONTENTS_IDLE_REFRESH_INTERVAL
        };
        if now.saturating_sub(last_check) < interval.as_secs() {
            continue;
        }

        let pins = LuaManifestPins::new(steam_path.to_string(), *app_id);
        if !pins.path_exists() {
            continue;
        }
        match pins.updates_are_enabled() {
            Ok(true) => candidates.push(*app_id),
            Ok(false) => crate::desk_log_debug!(
                "hubcap-updates",
                "Contents check skipped app_id={}: the user locked this game to a fixed version",
                app_id
            ),
            Err(error) => crate::desk_log_warn!(
                "hubcap-updates",
                "Contents check skipped app_id={}: {}",
                app_id,
                error
            ),
        }
    }
    candidates.sort_unstable();
    candidates
}
