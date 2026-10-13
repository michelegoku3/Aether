// ---------------------------------------------------------------------------
// Mirror types of the Rust background-synchronizer status snapshot
// (`core/hubcap_update_monitor.rs`, `#[serde(rename_all = "camelCase")]`).
//
// Extracted from the modal the same way `types/online.ts` absorbed
// the online panel types: the IPC contract lives in `types/`, the component
// lives in `modals/`.
// ---------------------------------------------------------------------------

/** Live state of one synchronizer lane. */
export interface LaneStatus {
  /** Queued tasks with the next scheduled attempt. */
  pending: PendingTaskInfo[];
  /**
   * Tasks abandoned after the bounded retry ladder (`MAX_TASK_ATTEMPTS`): no
   * longer retried on a timer. The next Steam-side change (or an app restart)
   * re-queues them. This used to be invisible everywhere: an unsynced pin
   * stayed silent.
   */
  unresolved?: PendingTaskInfo[];
  /** Tasks successfully completed since the monitor started. */
  processedCount: number;
  /** Unix epoch (seconds) of the last completed task. */
  lastRunEpoch: number | null;
  /** Last error message, kept until the next success. */
  lastError: string | null;
}

/** A queued (or abandoned) task in a lane. */
export interface PendingTaskInfo {
  /** `null` for non-per-app tasks (the Workshop sync isn't one). */
  appId: number | null;
  attempts: number;
  /** Unix epoch (seconds) of the next attempt, when known. */
  nextRetryEpoch: number | null;
}

/** Full snapshot exposed by `get_hubcap_monitor_status`. */
export interface MonitorStatus {
  running: boolean;
  startedEpoch: number | null;
  lastScanEpoch: number | null;
  steamPathConfigured: boolean;
  hubcapKeyConfigured: boolean;
  checkpointInitialized: boolean;
  pinSync: LaneStatus;
  pinRefresh: LaneStatus;
  repair: LaneStatus;
  workshop: LaneStatus;
}

/**
 * The four lanes, in popup listing order. Keeping them in a table (instead of
 * four near-identical JSX blocks) is what allows adding a lane without
 * touching the component.
 */
export interface SyncLaneDescriptor {
  /** Snapshot key + shown title. */
  lane: 'pinSync' | 'pinRefresh' | 'repair' | 'workshop';
  title: string;
  hint: string;
}

export const SYNC_LANES: readonly SyncLaneDescriptor[] = [
  {
    lane: 'pinSync',
    title: 'Pin sync (after Steam updates)',
    hint: 'Realigns commented pins of games with updates enabled and archives the new manifests.',
  },
  {
    lane: 'pinRefresh',
    title: 'Pin refresh (Hubcap contents diff)',
    hint: 'Periodically diffs Lua pins against the manifests Hubcap packages (free endpoint), stages missing manifests and realigns commented pins of updates-ON games.',
  },
  {
    lane: 'repair',
    title: 'Manifest repair (Lua changes)',
    hint: 'Local-first repair of a replaced or edited Lua. Requires a Hubcap key for manifests that are not on disk.',
  },
  {
    lane: 'workshop',
    title: 'Workshop staging',
    hint: 'Stages missing Workshop item manifests when the installed Workshop set changes.',
  },
] as const;
