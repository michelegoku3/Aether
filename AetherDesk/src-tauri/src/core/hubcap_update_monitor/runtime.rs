//! Explicit in-memory state of the four monitor lanes.

use super::scheduler::{pending_infos, PendingTask};
use super::status::{now_epoch, with_status, PendingTaskInfo};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Default)]
pub(super) struct MonitorRuntime {
    pub(super) pin_sync: BTreeMap<u32, PendingTask>,
    pub(super) pin_refresh: BTreeMap<u32, PendingTask>,
    pub(super) repairs: BTreeMap<u32, PendingTask>,
    pub(super) workshop: Option<PendingTask>,
    pub(super) last_pin_refresh_at: Option<Instant>,
}

impl MonitorRuntime {
    pub(super) fn publish_status(
        &self,
        steam_path_configured: bool,
        hubcap_key_configured: bool,
        checkpoint_initialized: bool,
    ) {
        with_status(|status| {
            status.last_scan_epoch = Some(now_epoch());
            status.steam_path_configured = steam_path_configured;
            status.hubcap_key_configured = hubcap_key_configured;
            status.checkpoint_initialized = checkpoint_initialized;
            status.pin_sync.pending = pending_infos(&self.pin_sync);
            status.pin_refresh.pending = pending_infos(&self.pin_refresh);
            status.repair.pending = pending_infos(&self.repairs);
            status.workshop.pending = self
                .workshop
                .as_ref()
                .map(|task| {
                    vec![PendingTaskInfo {
                        app_id: None,
                        attempts: task.attempts,
                        next_retry_epoch: Some(
                            now_epoch()
                                + task
                                    .next_attempt
                                    .saturating_duration_since(Instant::now())
                                    .as_secs(),
                        ),
                    }]
                })
                .unwrap_or_default();
        });
    }
}
