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
