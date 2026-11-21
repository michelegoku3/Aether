/**
 * Result of the `repair_appinfo_cache` command: where the old cache was moved
 * and how large it was, so the UI can tell the user what to keep.
 */
export interface AppInfoRepair {
  backupPath: string;
  bytes: number;
}

/**
 * IPC payload for the commands that write into the Steam folder. `true` only
 * after the user accepted the "Close Steam?" popup: the backend then terminates
 * Steam before touching its files.
 */
export type SteamKillConsentArgs = {
  allowSteamKill: boolean;
};
