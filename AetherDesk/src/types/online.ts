// ---------------------------------------------------------------------------
// Mirror types of the Rust online commands (serde rename_all = camelCase)
// ---------------------------------------------------------------------------

export type EngineKind = 'unity' | 'unreal' | 'generic';
export type ArchKind = 'x64' | 'x86';
export type PhotonFlavorKind = 'none' | 'realtime' | 'fusion';
export type OnlineStateKind = 'not_configured' | 'enabled' | 'broken';

export interface OnlineBackendReport {
  photon: PhotonFlavorKind;
  photonVoice: boolean;
  eos: boolean;
  playfab: boolean;
  coherence: boolean;
}

export interface OnlineConflict {
  kind: string;
  path: string;
}

export interface OnlineDetectionReport {
  gameRoot: string;
  engine: EngineKind;
  arch: ArchKind;
  gameExe: string | null;
  unityDataDir: string | null;
  steamApiDir: string | null;
  iniDir: string;
  backends: OnlineBackendReport;
  conflicts: OnlineConflict[];
  steamlessApplied: boolean;
  steamstubDetected: boolean;
  warnings: string[];
}

export interface OnlinePrerequisites {
  bundleOk: boolean;
  bundleVersion: string | null;
  steamApiDirWritable: boolean;
  errors: string[];
}

export interface OnlineRecord {
  appId: number;
  enabledAt: number;
  bundleVersion: string | null;
  ogAppId: number;
  spoofAppId: number;
  iniPath: string;
  steamApiPath: string;
  arch: ArchKind;
  backendsDeployed: string[];
  backupDir: string;
  overlayProxyPath: string | null;
}

export interface OnlinePlan {
  detection: OnlineDetectionReport;
  prerequisites: OnlinePrerequisites;
  current: OnlineRecord | null;
  notices: string[];
}

export interface OnlineStatus {
  state: OnlineStateKind;
  record: OnlineRecord | null;
}

export interface OnlineActionResult {
  success: boolean;
  message: string;
  record: OnlineRecord | null;
}

export interface OnlineEnableRequest {
  ogAppId: number;
  spoofAppId: number;
  verboseLog: boolean;
  emulateTicket: boolean;
  warnOverlayDisabled: boolean;
  sdr: boolean;
  loadOverlay: boolean;
  logOverlay: boolean;
  getStubbedLol: boolean;
  client: string;
  unlockAllDlc: boolean;
  deployPhoton: boolean;
  photon: { realtimeGuid: string; voiceGuid: string; fusionGuid: string };
  eos: { productId: string; sandboxId: string; deploymentId: string; clientId: string; clientSecret: string };
  playfab: { titleId: string; useShared: boolean };
  coherence: { runtimeKey: string; useShared: boolean };
  deployEosCustom: boolean;
  deployOverlayProxy: boolean;
}

export const emptyRequest = (ogAppId: number): OnlineEnableRequest => ({
  ogAppId,
  spoofAppId: 480,
  verboseLog: true,
  emulateTicket: false,
  warnOverlayDisabled: false,
  sdr: false,
  loadOverlay: true,
  logOverlay: false,
  getStubbedLol: false,
  client: '',
  unlockAllDlc: true,
  deployPhoton: false,
  photon: { realtimeGuid: '', voiceGuid: '', fusionGuid: '' },
  eos: { productId: '', sandboxId: '', deploymentId: '', clientId: '', clientSecret: '' },
  playfab: { titleId: '', useShared: false },
  coherence: { runtimeKey: '', useShared: false },
  deployEosCustom: false,
  deployOverlayProxy: true,
});

export const engineLabel = (engine: EngineKind): string => {
  switch (engine) {
    case 'unity': return 'Unity';
    case 'unreal': return 'Unreal';
    default: return 'Generic';
  }
};

export const photonFlavorLabel = (flavor: PhotonFlavorKind): string => {
  switch (flavor) {
    case 'fusion': return 'Fusion';
    default: return 'Realtime';
  }
};

export const backendChips = (backends: OnlineBackendReport): string[] => {
  const chips: string[] = [];
  if (backends.photon !== 'none') {
    chips.push(`Photon ${photonFlavorLabel(backends.photon)}${backends.photonVoice ? ' + Voice' : ''}`);
  }
  if (backends.eos) chips.push('EOS');
  if (backends.playfab) chips.push('PlayFab');
  if (backends.coherence) chips.push('coherence');
  if (chips.length === 0) chips.push('Steam P2P');
  return chips;
};

export const conflictLabel = (kind: string): string => {
  switch (kind) {
    case 'coldClientLoader': return 'ColdClientLoader';
    case 'steamFix': return 'SteamFix';
    case 'ofme': return 'OFME (online-fix.me)';
    case 'namedFixFile': return 'Fix file (winmm/...)';
    case 'proxyDll': return 'Proxy DLL';
    default: return kind;
  }
};

// ---------------------------------------------------------------------------
// Presence (aethercore.toml [presence]) — contratto Desk <-> DLL
// ---------------------------------------------------------------------------

/**
 * Token di dominio scritto in `aethercore.toml` come `[presence] default_mode`
 * e come nome degli array (`showonline_apps`, `aetheronline_apps`,
 * `exclude_apps`).
 *
 * È letto ANCHE da AetherDLL (`AetherCore/core/Settings.cpp`): la grafia tutta
 * minuscola non è una svista di casing e NON va "normalizzata" a `showOnline`.
 * Vedi `docs/shared_contracts.md` §7.
 */
export type PresenceDefaultMode = 'none' | 'showonline';

/**
 * Modalità di presenza effettiva di un gioco, come la risolve il popup ONLINE.
 * `'none'` copre sia l'opt-out esplicito (`exclude_apps`) sia l'assenza di
 * qualsiasi marker con policy di default `none`.
 */
export type AppPresenceMode = 'none' | 'showonline' | 'aetheronline';

/** Le opzioni del popup: le tre modalità più il pannello UCO2 (che non è una
 *  modalità di presenza ma una pipeline separata, e richiede `none`). */
export type OnlineOptionKey = AppPresenceMode | 'uco2';

/**
 * Argomenti IPC dei comandi presence.
 *
 * Le chiavi sono quelle che Tauri 2 accetta davvero: il proc-macro converte il
 * parametro Rust in lowerCamelCase e la lookup sul payload è su QUELLA chiave,
 * senza alcun fallback snake_case (`tauri 2.11.5`, `ipc/command.rs`). Tenere
 * le shape qui — invece che inline nei `invoke` — è ciò che rende impossibile
 * scrivere `{ showonline }` quando il backend si aspetta `{ showOnline }`.
 *
 * **`type`, non `interface`**: `invoke()` accetta `InvokeArgs =
 * Record<string, unknown> | ...`, e un `interface` non ha l'index signature
 * implicita → non è assegnabile (TS2345). Un type alias di object literal lo
 * è. Regola generale per tutti i contratti di argomenti IPC di questo progetto.
 */
export type PresenceToggleArgs = {
  appId: number;
  enabled: boolean;
};

/** `set_presence_default_mode(show_online: bool)` → chiave `showOnline`. */
export type SetPresenceDefaultModeArgs = {
  showOnline: boolean;
};
