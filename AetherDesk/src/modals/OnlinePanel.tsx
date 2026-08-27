import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { LibraryActionGame } from './LibraryGameActionsModal';
import { folderOf, openInFileManager, pathFromGameRoot, shortenPath } from '../util/paths';
import { useModalDismiss, useOverlayDismiss } from '../hooks/useModalDismiss';

// ---------------------------------------------------------------------------
// Mirror types of the Rust commands (serde rename_all = camelCase)
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

// Stili: classi .oc-*/.op-* in style.css (sezione "Online modals").
const emptyRequest = (ogAppId: number): OnlineEnableRequest => ({
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

const engineLabel = (engine: EngineKind): string => {
  switch (engine) {
    case 'unity': return 'Unity';
    case 'unreal': return 'Unreal';
    default: return 'Generic';
  }
};

const photonFlavorLabel = (flavor: PhotonFlavorKind): string => {
  switch (flavor) {
    case 'fusion': return 'Fusion';
    default: return 'Realtime';
  }
};

const backendChips = (backends: OnlineBackendReport): string[] => {
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

const conflictLabel = (kind: string): string => {
  switch (kind) {
    case 'coldClientLoader': return 'ColdClientLoader';
    case 'steamFix': return 'SteamFix';
    case 'ofme': return 'OFME (online-fix.me)';
    case 'namedFixFile': return 'Fix file (winmm/...)';
    case 'proxyDll': return 'Proxy DLL';
    default: return kind;
  }
};

interface OnlinePanelProps {
  game: LibraryActionGame;
  onClose: () => void;
}

export const OnlinePanel = ({ game, onClose }: OnlinePanelProps) => {
  const [plan, setPlan] = useState<OnlinePlan | null>(null);
  const [status, setStatus] = useState<OnlineStatus | null>(null);
  const [request, setRequest] = useState<OnlineEnableRequest>(() => emptyRequest(Number(game.appId) || 0));
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; kind: 'info' | 'success' | 'error' } | null>(null);

  const refresh = useCallback(async () => {
    try {
      const appId = Number(game.appId);
      const [planResult, statusResult, savedRequest] = await Promise.all([
        invoke<OnlinePlan>('plan_online', { appId }),
        invoke<OnlineStatus>('get_online_status', { appId }),
        invoke<OnlineEnableRequest | null>('get_online_preferences', { appId }),
      ]);
      setPlan(planResult);
      setStatus(statusResult);
      setRequest((previous) => {
        const next = savedRequest ?? {
          ...previous,
          ogAppId: previous.ogAppId || appId || 0,
        };
        if (
          !savedRequest
          && planResult.detection.steamstubDetected
          && !planResult.detection.steamlessApplied
        ) {
          return { ...next, getStubbedLol: true };
        }
        return next;
      });
    } catch (err) {
      setMessage({ text: `Plan unavailable: ${err}`, kind: 'error' });
    } finally {
      setLoading(false);
    }
  }, [game.appId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // ESC chiude il popup (uniforme con tutti gli altri modali); il click
  // fuori è già gestito dall'overlay qui sotto (entrambi bloccati quando busy).
  // stopPropagation centralizzato in useOverlayDismiss: questo pannello è
  // annidato dentro l'overlay del popup Modify, e il click fuori deve
  // chiudere solo lui facendo tornare al popup di scelta online.
  useModalDismiss(onClose, busy);
  const handleOverlayClick = useOverlayDismiss(onClose, busy);

  const handleEnable = async () => {
    setBusy(true);
    setMessage({ text: 'Enabling...', kind: 'info' });
    try {
      const result = await invoke<OnlineActionResult>('enable_online', {
        appId: Number(game.appId),
        request,
      });
      setMessage({ text: result.message, kind: result.success ? 'success' : 'error' });
      await refresh();
    } catch (err) {
      setMessage({ text: `Enable failed: ${err}`, kind: 'error' });
    } finally {
      setBusy(false);
    }
  };

  const handleDisable = async () => {
    setBusy(true);
    setMessage({ text: 'Disabling...', kind: 'info' });
    try {
      const result = await invoke<OnlineActionResult>('disable_online', { appId: Number(game.appId) });
      setMessage({ text: result.message, kind: result.success ? 'success' : 'error' });
      await refresh();
    } catch (err) {
      setMessage({ text: `Disable failed: ${err}`, kind: 'error' });
    } finally {
      setBusy(false);
    }
  };

  // Restores defaults and explicitly clears the persisted per-game choices.
  const handleReset = async () => {
    const appId = Number(game.appId) || 0;
    try {
      await invoke('clear_online_preferences', { appId });
      setRequest(emptyRequest(appId));
      setMessage(null);
    } catch (err) {
      setMessage({ text: `Reset failed: ${err}`, kind: 'error' });
    }
  };

  const set = <K extends keyof OnlineEnableRequest>(key: K, value: OnlineEnableRequest[K]) =>
    setRequest((prev) => ({ ...prev, [key]: value }));

  const setPhoton = (key: 'realtimeGuid' | 'voiceGuid' | 'fusionGuid', value: string) =>
    setRequest((prev) => ({ ...prev, photon: { ...prev.photon, [key]: value } }));

  const setEos = (key: 'productId' | 'sandboxId' | 'deploymentId' | 'clientId' | 'clientSecret', value: string) =>
    setRequest((prev) => ({ ...prev, eos: { ...prev.eos, [key]: value } }));

  const enabled = status?.state === 'enabled';
  const broken = status?.state === 'broken';
  const blocked = !plan?.prerequisites.bundleOk || plan?.prerequisites.errors.length > 0;
  const bundleUpdate = enabled
    && !!plan?.prerequisites.bundleVersion
    && !!status?.record?.bundleVersion
    && plan.prerequisites.bundleVersion !== status.record.bundleVersion;

  const openFolder = async (folder: string) => {
    try {
      await openInFileManager(folder);
    } catch (err) {
      setMessage({ text: `Could not open the folder: ${err}`, kind: 'error' });
    }
  };

  // Checkbox row: description on the left, custom checkbox (same style as
  // Apply Crack) on the far right.
  const checkboxRow = (checked: boolean, disabled: boolean, onChange: (v: boolean) => void, description: string) => (
    <div className="op-checkbox-row">
      <span className="op-checkbox-desc">{description}</span>
      <label className="crack-checkbox-label op-checkbox-label">
        <input
          type="checkbox"
          className="crack-checkbox-input"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          disabled={disabled}
        />
        <span className="crack-checkbox-box"></span>
      </label>
    </div>
  );

  return (
    <div className="modal-overlay" onClick={handleOverlayClick}>
      <div
        className="modal-container op-container"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="op-header">
          <h3 className="op-header-title">Enable Online: {game.name} ({game.appId})</h3>
          <button type="button" className="op-close" onClick={onClose} disabled={busy} aria-label="Close">×</button>
        </div>

        <div className="op-body">
          {loading ? (
            <div className="op-muted">Analyzing the game...</div>
          ) : (
            <>
              {/* Prerequisites errors */}
              {plan && plan.prerequisites.errors.length > 0 && (
                <div className="op-box">
                  {plan.prerequisites.errors.map((error, index) => (
                    <div key={index} className="op-error">⚠ {error}</div>
                  ))}
                </div>
              )}

              {/* Detection */}
              {plan && (
                <div className="op-box">
                  <div className="op-row">
                    <span className="op-label">UCO2</span>
                    <span className={plan.prerequisites.bundleOk ? 'op-ok' : 'op-error'}>
                      {plan.prerequisites.bundleOk
                        ? `${plan.prerequisites.bundleVersion ?? 'Available'} ✓`
                        : 'N/A ✗'}
                    </span>
                  </div>
                  <div className="op-row">
                    <span className="op-label">Writable folder</span>
                    <span className={plan.prerequisites.steamApiDirWritable ? 'op-ok' : 'op-error'}>
                      {plan.prerequisites.steamApiDirWritable ? 'Yes ✓' : 'No ✗'}
                    </span>
                  </div>
                  <div className="op-row">
                    <span className="op-label">Engine</span>
                    <span>{engineLabel(plan.detection.engine)} · {plan.detection.arch === 'x64' ? '64-bit' : '32-bit'}</span>
                  </div>
                  {plan.detection.gameExe && (
                    <div className="op-row">
                      <span className="op-label">Executable</span>
                      <span
                        className="settings-path settings-path--wide settings-path-clickable op-path-link"
                        title={plan.detection.gameExe}
                        onClick={() => openFolder(folderOf(plan.detection.gameExe!))}
                      >
                        {shortenPath(pathFromGameRoot(plan.detection.gameRoot, plan.detection.gameExe))}
                      </span>
                    </div>
                  )}
                  {plan.detection.steamApiDir && (
                    <div className="op-row">
                      <span className="op-label">DLL location</span>
                      <span
                        className="settings-path settings-path--wide settings-path-clickable op-path-link"
                        title={plan.detection.steamApiDir}
                        onClick={() => openFolder(plan.detection.steamApiDir!)}
                      >
                        {shortenPath(`${pathFromGameRoot(plan.detection.gameRoot, plan.detection.steamApiDir)}\\`)}
                      </span>
                    </div>
                  )}
                  <div className="op-row">
                    <span className="op-label">Backend</span>
                    <span>{backendChips(plan.detection.backends).map((chip) => (
                      <span key={chip} className="op-chip op-chip--off">{chip}</span>
                    ))}</span>
                  </div>
                  {plan.detection.conflicts.length > 0 && (
                    <div className="op-row">
                      <span className="op-label">Conflicts</span>
                      <span className="op-warn">
                        {plan.detection.conflicts.map((c) => conflictLabel(c.kind)).join(', ')}: will be neutralized (reversible)
                      </span>
                    </div>
                  )}
                  {/* Notices: always the last lines of this box */}
                  {plan.notices.length > 0 && (
                    <div className="op-notices">
                      {plan.notices.map((notice, index) => (
                        <div key={index} className="op-warn">⚠ {notice}</div>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {/* AppID */}
              {plan && (
                <div className="op-box">
                  <div className="op-section-title">AppID</div>
                  <div className="op-row">
                    <span className="op-label">Spoof</span>
                    <input
                      className="op-input"
                      type="text"
                      inputMode="numeric"
                      pattern="[0-9]*"
                      value={request.spoofAppId}
                      onChange={(e) => set('spoofAppId', Number(e.target.value) || 480)}
                      disabled={enabled}
                    />
                  </div>
                  <div className="op-row">
                    <span className="op-label">ogAppID</span>
                    <input
                      className="op-input"
                      type="text"
                      inputMode="numeric"
                      pattern="[0-9]*"
                      value={request.ogAppId}
                      onChange={(e) => set('ogAppId', Number(e.target.value) || 0)}
                      disabled={enabled}
                    />
                  </div>
                  <div className="op-row">
                    <span className="op-label">Client (old-SDK)</span>
                    <input
                      className="op-input"
                      type="text"
                      value={request.client}
                      onChange={(e) => set('client', e.target.value)}
                      disabled={enabled}
                      placeholder="017"
                    />
                  </div>
                  <div className="op-note-warn">⚠ Write 017 for old-SDK games that crash on startup (e.g. SpeedRunners).</div>
                </div>
              )}

              {/* Photon */}
              {plan && plan.detection.backends.photon !== 'none' && (
                <div className="op-box">
                  <div className="op-section-title">Photon</div>
                  {checkboxRow(
                    request.deployPhoton,
                    enabled,
                    (v) => set('deployPhoton', v),
                    'Deploy Photon plugin'
                  )}
                  {request.deployPhoton && (
                    <div className="op-sub">
                      {plan.detection.backends.photon === 'fusion' ? (
                        <div className="op-row">
                          <span className="op-label">Fusion App GUID</span>
                          <input className="op-input" value={request.photon.fusionGuid} onChange={(e) => setPhoton('fusionGuid', e.target.value)} disabled={enabled} placeholder="app-id-xxxx" />
                        </div>
                      ) : (
                        <>
                          <div className="op-row">
                            <span className="op-label">Realtime App GUID</span>
                            <input className="op-input" value={request.photon.realtimeGuid} onChange={(e) => setPhoton('realtimeGuid', e.target.value)} disabled={enabled} placeholder="app-id-xxxx" />
                          </div>
                          {plan.detection.backends.photonVoice && (
                            <div className="op-row">
                              <span className="op-label">Voice App GUID</span>
                              <input className="op-input" value={request.photon.voiceGuid} onChange={(e) => setPhoton('voiceGuid', e.target.value)} disabled={enabled} placeholder="app-id-xxxx" />
                            </div>
                          )}
                        </>
                      )}
                    </div>
                  )}
                </div>
              )}

              {/* EOS */}
              {plan && plan.detection.backends.eos && (
                <div className="op-box">
                  <div className="op-section-title">Epic Online Services</div>
                  {checkboxRow(
                    request.deployEosCustom,
                    enabled,
                    (v) => set('deployEosCustom', v),
                    'Deploy EOS_custom'
                  )}
                  {request.deployEosCustom && (
                    <div className="op-sub">
                      {(['productId', 'sandboxId', 'deploymentId', 'clientId', 'clientSecret'] as const).map((key) => (
                        <div className="op-row" key={key}>
                          <span className="op-label">{key}</span>
                          <input className="op-input" value={request.eos[key]} onChange={(e) => setEos(key, e.target.value)} disabled={enabled} />
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {/* PlayFab */}
              {plan && plan.detection.backends.playfab && (
                <div className="op-box">
                  <div className="op-section-title">PlayFab</div>
                  <div className="op-row">
                    <span className="op-label">TitleId (yours)</span>
                    <input
                      className="op-input"
                      value={request.playfab.titleId}
                      onChange={(e) => set('playfab', { ...request.playfab, titleId: e.target.value })}
                      disabled={enabled || request.playfab.useShared}
                      placeholder="XXXXX or SHARED (empty = inert plugin)"
                    />
                  </div>
                  {checkboxRow(
                    request.playfab.useShared,
                    enabled,
                    (v) => set('playfab', { ...request.playfab, useShared: v }),
                    'Use the SHARED community TitleId (1D861F — everyone must match)',
                  )}
                </div>
              )}

              {/* coherence */}
              {plan && plan.detection.backends.coherence && (
                <div className="op-box">
                  <div className="op-section-title">coherence</div>
                  <div className="op-row">
                    <span className="op-label">Runtime key</span>
                    <input className="op-input" value={request.coherence.runtimeKey} onChange={(e) => set('coherence', { ...request.coherence, runtimeKey: e.target.value })} disabled={enabled || request.coherence.useShared} placeholder="your project (schema uploaded)" />
                  </div>
                  {checkboxRow(
                    request.coherence.useShared,
                    enabled,
                    (v) => set('coherence', { ...request.coherence, useShared: v }),
                    'Use the SHARED community project (no account, availability not guaranteed)'
                  )}
                </div>
              )}

              {/* Settings */}
              {plan && (
                <div className="op-box">
                  <div className="op-section-title">Settings</div>
                  {checkboxRow(
                    request.verboseLog,
                    enabled,
                    (v) => set('verboseLog', v),
                    'Verbose UCO2 logs'
                  )}
                  {checkboxRow(
                    request.emulateTicket,
                    enabled,
                    (v) => set('emulateTicket', v),
                    'Emulate auth ticket'
                  )}
                  {checkboxRow(
                    request.warnOverlayDisabled,
                    enabled,
                    (v) => set('warnOverlayDisabled', v),
                    'Warn when Steam overlay is disabled'
                  )}
                  {checkboxRow(
                    request.loadOverlay,
                    enabled,
                    (v) => set('loadOverlay', v),
                    'Load Steam overlay renderer (LoadOverlay)'
                  )}
                  {checkboxRow(
                    request.logOverlay,
                    enabled,
                    (v) => set('logOverlay', v),
                    'Write steam_overlay.log (LogOverlay)'
                  )}
                  {checkboxRow(
                    request.getStubbedLol,
                    enabled,
                    (v) => set('getStubbedLol', v),
                    'SteamStub runtime hook (GetStubbedLol)'
                  )}
                  {checkboxRow(
                    request.unlockAllDlc,
                    enabled,
                    (v) => set('unlockAllDlc', v),
                    'Unlock all DLC'
                  )}
                  {checkboxRow(
                    request.sdr,
                    enabled,
                    (v) => set('sdr', v),
                    'Steam Datagram Relay'
                  )}
                  {plan.detection.engine !== 'generic' && plan.detection.arch === 'x64' && checkboxRow(
                    request.deployOverlayProxy,
                    enabled,
                    (v) => set('deployOverlayProxy', v),
                    plan.detection.engine === 'unity'
                      ? 'Early overlay proxy (version.dll)'
                      : 'Early overlay proxy (XINPUT1_3.dll)',
                  )}
                </div>
              )}

              {/* Result message */}
              {message && (
                <div className={`op-box op-message op-message--${message.kind}`}>
                  {message.text}
                </div>
              )}
            </>
          )}
        </div>

        {/* I due tasti non cambiano mai posizione: Enable/Disable a sinistra,
            Reset a destra. Reset diventa non cliccabile quando UCO2 è attivo. */}
        <div className="op-footer">
          {bundleUpdate && (
            <button
              type="button"
              className={`modal-btn op-footer-btn op-btn${blocked || busy ? ' op-btn--disabled' : ''}`}
              onClick={handleEnable}
              disabled={blocked || busy}
              title={`Update deployed files to ${plan?.prerequisites.bundleVersion ?? 'the current bundle'}`}
            >
              {busy ? 'Updating...' : `Update ${plan?.prerequisites.bundleVersion ?? ''}`.trim()}
            </button>
          )}
          <button
            type="button"
            className={`modal-btn op-footer-btn ${enabled || broken ? 'op-btn--ghost' : 'op-btn'}${(blocked || busy) ? ' op-btn--disabled' : ''}`}
            onClick={enabled || broken ? handleDisable : handleEnable}
            disabled={enabled || broken ? busy : blocked || busy}
            title={!enabled && !broken && blocked ? 'Resolve the missing prerequisites first' : undefined}
          >
            {busy ? (enabled || broken ? 'Disabling...' : 'Enabling...') : enabled || broken ? 'Disable' : 'Enable'}
          </button>
          <button
            type="button"
            className={`modal-btn op-footer-btn op-btn--ghost${enabled || broken || busy ? ' op-btn--disabled' : ''}`}
            onClick={handleReset}
            disabled={enabled || broken || busy}
          >
            Reset
          </button>
        </div>
      </div>
    </div>
  );
};
