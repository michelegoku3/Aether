export interface DeskUpdateInfo {
  update_available?: boolean;
  is_test?: boolean;
  installed_version?: string;
}

export interface DllUpdateInfo {
  update_available?: boolean;
  is_test?: boolean;
  installed_version?: string;
}

export type HubcapKeyState = 'active' | 'inactive' | 'unknown';

export interface HubcapUsageStats {
  usage: number;
  limit: number;
  /** Definitive answer from `/user/stats`; null when the provider could not be asked. */
  active?: boolean | null;
  reason?: string;
  /** Short digest of the key this answer belongs to (never the key itself). */
  keyFingerprint?: string;
}

export interface HubcapUsage {
  usage: number;
  limit: number;
  hasKey: boolean;
  active: boolean | null;
  reason?: string;
  keyFingerprint?: string;
  /**
   * This answer was requested quietly: it may update the badge but must not
   * raise the global expired-key warning (the caller already reported it).
   */
  quiet?: boolean;
}

/** Options for one quota refresh. */
export interface HubcapRefreshOptions {
  /**
   * Ask for the verdict without publishing the global expired-key warning.
   * Used right after a Settings save that already told the user the key is not
   * active, so the badge updates without a modal on top of that message.
   */
  quiet?: boolean;
}
