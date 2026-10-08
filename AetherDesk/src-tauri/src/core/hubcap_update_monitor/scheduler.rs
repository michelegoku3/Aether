//! Shared queue, retry and unresolved-task policy for monitor lanes.

use super::status::{now_epoch, with_status};
use super::{LaneStatus, MonitorStatusSnapshot, PendingTaskInfo};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub(crate) const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(30);
pub(crate) const MAX_RETRY_DELAY: Duration = Duration::from_secs(30 * 60);
pub(crate) const MAX_TASK_ATTEMPTS: u32 = 6;

#[derive(Debug, Clone)]
pub(crate) struct PendingTask {
    pub(crate) attempts: u32,
    pub(crate) next_attempt: Instant,
}

impl PendingTask {
    pub(crate) fn due_now() -> Self {
        Self {
            attempts: 0,
            next_attempt: Instant::now(),
        }
    }
}

pub(crate) fn retry_delay(attempts: u32) -> Duration {
    let multiplier = 1u64 << attempts.min(6);
    INITIAL_RETRY_DELAY
        .checked_mul(u32::try_from(multiplier).unwrap_or(u32::MAX))
        .unwrap_or(MAX_RETRY_DELAY)
        .min(MAX_RETRY_DELAY)
}

/// Removes the first task whose backoff has expired. BTreeMap ordering makes
/// selection deterministic by AppID while keeping one task per lane per poll.
pub(super) fn take_ready(
    pending: &mut BTreeMap<u32, PendingTask>,
) -> Option<(u32, PendingTask)> {
    let now = Instant::now();
    let ready = pending
        .iter()
        .find(|(_, task)| task.next_attempt <= now)
        .map(|(app_id, _)| *app_id)?;
    pending.remove(&ready).map(|task| (ready, task))
}

/// Requeues one failure or gives up after the bounded retry ladder.
pub(crate) fn reschedule(
    pending: &mut BTreeMap<u32, PendingTask>,
    app_id: u32,
    task: &PendingTask,
) -> bool {
    let attempts = task.attempts.saturating_add(1);
    if attempts >= MAX_TASK_ATTEMPTS {
        pending.remove(&app_id);
        crate::desk_log_warn!(
            "hubcap-updates",
            "Task abandoned app_id={} attempts={} max_attempts={}; it will be queued again on the next Steam-side change",
            app_id,
            attempts,
            MAX_TASK_ATTEMPTS
        );
        return false;
    }
    let delay = retry_delay(attempts);
    crate::desk_log_warn!(
        "hubcap-updates",
        "Task deferred app_id={} attempts={} retry_in_secs={}",
        app_id,
        attempts,
        delay.as_secs()
    );
    pending.insert(
        app_id,
        PendingTask {
            attempts,
            next_attempt: Instant::now() + delay,
        },
    );
    true
}

pub(super) fn mark_unresolved(
    lane: fn(&mut MonitorStatusSnapshot) -> &mut LaneStatus,
    app_id: u32,
    attempts: u32,
) {
    with_status(|status| {
        let lane = lane(status);
        lane.unresolved.retain(|info| info.app_id != Some(app_id));
        lane.unresolved.push(PendingTaskInfo {
            app_id: Some(app_id),
            attempts,
            next_retry_epoch: None,
        });
    });
}

pub(super) fn clear_unresolved(
    lane: fn(&mut MonitorStatusSnapshot) -> &mut LaneStatus,
    app_id: u32,
) {
    with_status(|status| {
        lane(status)
            .unresolved
            .retain(|info| info.app_id != Some(app_id));
    });
}

pub(super) fn pending_infos(
    pending: &BTreeMap<u32, PendingTask>,
) -> Vec<PendingTaskInfo> {
    pending
        .iter()
        .map(|(app_id, task)| PendingTaskInfo {
            app_id: Some(*app_id),
            attempts: task.attempts,
            next_retry_epoch: Some(
                now_epoch()
                    + task
                        .next_attempt
                        .saturating_duration_since(Instant::now())
                        .as_secs(),
            ),
        })
        .collect()
}
