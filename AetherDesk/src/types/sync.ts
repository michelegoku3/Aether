// ---------------------------------------------------------------------------
// Mirror types of the Rust background-synchronizer status snapshot
// (`core/hubcap_update_monitor.rs`, `#[serde(rename_all = "camelCase")]`).
//
// Estratti dal modale nello stesso modo in cui `types/online.ts` ha assorbito
// i tipi del pannello online: il contratto IPC vive in `types/`, il componente
// vive in `modals/`.
// ---------------------------------------------------------------------------

/** Stato live di una corsia del sincronizzatore. */
export interface LaneStatus {
  /** Task in coda con il prossimo tentativo programmato. */
  pending: PendingTaskInfo[];
  /**
   * Task abbandonati dopo la retry ladder limitata (`MAX_TASK_ATTEMPTS`): non
   * vengono più ritentati a timer. Il prossimo cambiamento lato Steam (o un
   * riavvio dell'app) li rimette in coda. È l'informazione che prima non era
   * visibile da nessuna parte: un pin non sincronizzato restava silenzioso.
   */
  unresolved?: PendingTaskInfo[];
  /** Task completati con successo dall'avvio del monitor. */
  processedCount: number;
  /** Epoch unix (secondi) dell'ultimo task completato. */
  lastRunEpoch: number | null;
  /** Ultimo messaggio di errore, conservato fino al prossimo successo. */
  lastError: string | null;
}

/** Un task in coda (o abbandonato) in una corsia. */
export interface PendingTaskInfo {
  /** `null` per i task non per-app (il sync Workshop non lo è). */
  appId: number | null;
  attempts: number;
  /** Epoch unix (secondi) del prossimo tentativo, quando noto. */
  nextRetryEpoch: number | null;
}

/** Snapshot completo esposto da `get_hubcap_monitor_status`. */
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
 * Le quattro corsie, nell'ordine in cui il popup le elenca. Tenerle in una
 * tabella (invece di quattro blocchi JSX quasi identici) è ciò che permette di
 * aggiungere una corsia senza toccare il componente.
 */
export interface SyncLaneDescriptor {
  /** Chiave dello snapshot + titolo mostrato. */
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
