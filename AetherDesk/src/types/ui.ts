export type StatusType = 'info' | 'success' | 'error';

export interface StatusMessage {
  text: string;
  type: StatusType;
}

export const emptyStatus = (): StatusMessage => ({ text: '', type: 'info' });

/**
 * Stato installazione/versione di AetherDLL, condiviso App → MainContent → AetherView.
 * La versione è letta dal backend direttamente dalla version resource PE dentro i
 * .dll della cartella Steam (nessun file esterno di versionamento).
 */
export interface DllStatusInfo {
  isInstalled: boolean;
  installedVersion: string;
  isSteamBlocked: boolean;
}
