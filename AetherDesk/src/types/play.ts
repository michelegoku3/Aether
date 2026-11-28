/** Process verdict returned by the pre-launch RAM optimizer. */
export type RamVerdict = 'close' | 'keep';

export interface RamProcessEntry {
  pid: number;
  name: string;
  exe: string;
  /** Working set in kilobytes. */
  memoryKb: number;
  user: string;
  verdict: RamVerdict;
  reason: string;
}

export interface SystemMemoryInfo {
  totalKb: number;
  usedKb: number;
  availableKb: number;
}

export interface RamPreview {
  entries: RamProcessEntry[];
  memory: SystemMemoryInfo;
}

export interface RamOptimizeReport {
  /** Processes that were actually terminated (including graceful WM_CLOSE exits). */
  closed: string[];
  /** Close candidates that could not be terminated. */
  failed: string[];
  /** Kept processes, `name (reason)`, sorted. */
  kept: string[];
  /** Working set of `closed`, in kilobytes. */
  reclaimedKb: number;
  /** Close candidates that had already exited before the optimizer signalled them. */
  gone: string[];
}

export interface PlayGameReport {
  report: RamOptimizeReport;
  launched: boolean;
  message: string;
}

/** Minimum memory (MB) for a process to be considered a close candidate. */
export const RAM_DEFAULT_MIN_MB = 150;

/** Preset memory thresholds offered in PlayGameModal. */
export const RAM_THRESHOLD_OPTIONS = [100, 150, 250, 500] as const;

export const formatMb = (kb: number): string => `${Math.round(kb / 1024)} MB`;

export const formatGb = (kb: number): string => `${(kb / (1024 * 1024)).toFixed(1)} GB`;
