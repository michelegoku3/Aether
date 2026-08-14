export type StatusType = 'info' | 'success' | 'error';

export interface StatusMessage {
  text: string;
  type: StatusType;
}

export const emptyStatus = (): StatusMessage => ({ text: '', type: 'info' });

/**
 * AetherDLL install/version state, shared App → MainContent → AetherView.
 * Version is read by the backend from the PE version resource inside the
 * Steam-folder DLLs (no external versioning file).
 */
export interface DllStatusInfo {
  isInstalled: boolean;
  installedVersion: string;
  isSteamBlocked: boolean;
}
