/**
 * Result of the `repair_appinfo_cache` command: where the old cache was moved
 * and how large it was, so the UI can tell the user what to keep.
 */
export interface AppInfoRepair {
  backupPath: string;
  bytes: number;
}
