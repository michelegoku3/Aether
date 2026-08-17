import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { LibraryActionGame } from './LibraryGameActionsModal';
import { folderOf, openInFileManager, pathFromGameRoot, shortenPath } from '../util/paths';

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
  photon: { realtimeGuid: string; voiceGuid: string; fusionGuid: string };
  eos: { productId: string; sandboxId: string; deploymentId: string; clientId: string; clientSecret: string };
  playfab: { titleId: string };
  coherence: { runtimeKey: string; useShared: boolean };
  deployEosCustom: boolean;
}

// ---------------------------------------------------------------------------
// Styles (component-scoped, no impact on the rest of the app)
// ---------------------------------------------------------------------------

const styles = {
  body: { padding: '16px 20px', overflowY: 'auto' as const, maxHeight: '62vh' },
  section: { marginBottom: '14px' },
  sectionTitle: { fontSize: '12px', fontWeight: 700, textTransform: 'uppercase' as const, letterSpacing: '0.06em', opacity: 0.65, marginBottom: '8px' },
  row: { display: 'flex' as const, gap: '10px', alignItems: 'center', marginBottom: '8px', flexWrap: 'wrap' as const },
  label: { fontSize: '12px', opacity: 0.75, minWidth: '150px' },
  input: { background: '#1b1b1f', border: '1px solid #2c2c31', borderRadius: '6px', color: '#eee', padding: '6px 8px', fontSize: '13px', flex: 1, minWidth: '160px' },
  chip: { padding: '2px 10px', borderRadius: '10px', fontSize: '12px', fontWeight: 600 },
  chipEnabled: { background: '#16351f', color: '#6fdb8c', border: '1px solid #2c6e42' },
  chipOff: { background: '#2a2a2e', color: '#9a9aa0', border: '1px solid #3a3a40' },
  chipBroken: { background: '#3a1d1d', color: '#e07b7b', border: '1px solid #6e2c2c' },
  box: { background: '#17171b', border: '1px solid #232329', borderRadius: '8px', padding: '10px 12px', marginBottom: '8px', fontSize: '12px' },
  warn: { color: '#e0b06a' },
  error: { color: '#e07b7b' },
  ok: { color: '#6fdb8c' },
  muted: { opacity: 0.6 },
  pathLink: { color: '#8ab4f8', cursor: 'pointer', textDecoration: 'underline', wordBreak: 'break-all' as const },
  footer: { display: 'flex' as const, gap: '10px', justifyContent: 'center' as const, padding: '12px 20px', borderTop: '1px solid #232329' },
  footerBtn: { flex: 1, maxWidth: 160, minWidth: 140, padding: '8px 16px', fontSize: '13px', fontWeight: 600, cursor: 'pointer' },
  btn: { background: '#2c6e42', color: '#fff', border: 'none', borderRadius: '8px' },
  btnDisabled: { opacity: 0.45, cursor: 'not-allowed' },
  btnGhost: { background: '#232329', color: '#ddd', border: '1px solid #2f2f35', borderRadius: '8px' },
  btnDanger: { background: '#5a2323', color: '#ffd9d9', border: 'none', borderRadius: '8px' },
  header: { display: 'flex' as const, justifyContent: 'space-between' as const, alignItems: 'center' as const, padding: '14px 20px', borderBottom: '1px solid #232329' },
  headerTitle: { margin: 0, fontSize: '16px', fontWeight: 700 },
  close: { background: 'none', border: 'none', color: '#999', fontSize: '20px', cursor: 'pointer', lineHeight: 1 },
  checkboxRow: { display: 'flex' as const, alignItems: 'center' as const, justifyContent: 'space-between' as const, gap: '12px', marginBottom: '8px', fontSize: '13px' },
  checkboxDesc: { flex: 1, fontSize: '12px', opacity: 0.85 },
};

const emptyRequest = (ogAppId: number): OnlineEnableRequest => ({
  ogAppId,
  spoofAppId: 480,
  photon: { realtimeGuid: '', voiceGuid: '', fusionGuid: '' },
  eos: { productId: '', sandboxId: '', deploymentId: '', clientId: '', clientSecret: '' },
  playfab: { titleId: '' },
  coherence: { runtimeKey: '', useShared: false },
  deployEosCustom: false,
});

const engineLabel = (engine: EngineKind): string => {
  switch (engine) {
    case 'unity': return 'Unity';
    case 'unreal': return 'Unreal';
    default: return 'Generic';
  }
};

const backendChips = (backends: OnlineBackendReport): string[] => {
  const chips: string[] = [];
  if (backends.photon !== 'none') {
    chips.push(`Photon ${backends.photon}${backends.photonVoice ? ' + Voice' : ''}`);
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
    case 'onlineFix': return 'OnlineFix';
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
      const [planResult, statusResult] = await Promise.all([
        invoke<OnlinePlan>('plan_online', { appId: Number(game.appId) }),
        invoke<OnlineStatus>('get_online_status', { appId: Number(game.appId) }),
      ]);
      setPlan(planResult);
      setStatus(statusResult);
      // Pre-fill the EOS default from detection (UCO2 is the preferred fix).
      setRequest((prev) => ({
        ...prev,
        deployEosCustom: planResult.detection.backends.eos ? prev.deployEosCustom || true : prev.deployEosCustom,
        ogAppId: prev.ogAppId || Number(game.appId) || 0,
      }));
    } catch (err) {
      setMessage({ text: `Plan unavailable: ${err}`, kind: 'error' });
    } finally {
      setLoading(false);
    }
  }, [game.appId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

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

  // Restores the form to its initial values.
  const handleReset = () => {
    setRequest(emptyRequest(Number(game.appId) || 0));
    setMessage(null);
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

  const openFolder = async (folder: string) => {
    try {
      await openInFileManager(folder);
    } catch (err) {
      setMessage({ text: `Could not open the folder: ${err}`, kind: 'error' });
    }
  };

  return (
    <div className="modal-overlay" onClick={busy ? undefined : onClose}>
      <div
        className="modal-container"
        style={{ width: 720, maxHeight: '86vh' }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={styles.header}>
          <h3 style={styles.headerTitle}>Enable Online: {game.name} ({game.appId})</h3>
          <button type="button" style={styles.close} onClick={onClose} disabled={busy} aria-label="Close">×</button>
        </div>

        <div style={styles.body}>
          {loading ? (
            <div style={styles.muted}>Analyzing the game...</div>
          ) : (
            <>
              {/* Prerequisites errors */}
              {plan && plan.prerequisites.errors.length > 0 && (
                <div style={styles.box}>
                  {plan.prerequisites.errors.map((error, index) => (
                    <div key={index} style={styles.error}>⚠ {error}</div>
                  ))}
                </div>
              )}

              {/* Detection */}
              {plan && (
                <div style={styles.box}>
                  <div style={styles.row}>
                    <span style={styles.label}>UCOnline2 bundle</span>
                    <span style={plan.prerequisites.bundleOk ? styles.ok : styles.error}>
                      {plan.prerequisites.bundleOk
                        ? plan.prerequisites.bundleVersion ?? 'Available'
                        : 'N/A ✗'}
                    </span>
                  </div>
                  <div style={styles.row}>
                    <span style={styles.label}>Writable game folder</span>
                    <span style={plan.prerequisites.steamApiDirWritable ? styles.ok : styles.error}>
                      {plan.prerequisites.steamApiDirWritable ? 'Yes ✓' : 'No ✗'}
                    </span>
                  </div>
                  <div style={styles.row}>
                    <span style={styles.label}>Engine</span>
                    <span>{engineLabel(plan.detection.engine)} · {plan.detection.arch === 'x64' ? '64-bit' : '32-bit'}</span>
                  </div>
                  {plan.detection.gameExe && (
                    <div style={styles.row}>
                      <span style={styles.label}>Executable</span>
                      <span
                        style={styles.pathLink}
                        title={plan.detection.gameExe}
                        onClick={() => openFolder(folderOf(plan.detection.gameExe!))}
                      >
                        {shortenPath(pathFromGameRoot(plan.detection.gameRoot, plan.detection.gameExe))}
                      </span>
                    </div>
                  )}
                  {plan.detection.steamApiDir && (
                    <div style={styles.row}>
                      <span style={styles.label}>DLL location</span>
                      <span
                        style={styles.pathLink}
                        title={plan.detection.steamApiDir}
                        onClick={() => openFolder(plan.detection.steamApiDir!)}
                      >
                        {shortenPath(pathFromGameRoot(plan.detection.gameRoot, plan.detection.steamApiDir))}
                      </span>
                    </div>
                  )}
                  <div style={styles.row}>
                    <span style={styles.label}>Backend</span>
                    <span>{backendChips(plan.detection.backends).map((chip) => (
                      <span key={chip} style={{ ...styles.chip, ...styles.chipOff, marginRight: 6 }}>{chip}</span>
                    ))}</span>
                  </div>
                  {plan.detection.conflicts.length > 0 && (
                    <div style={styles.row}>
                      <span style={styles.label}>Conflicts</span>
                      <span style={styles.warn}>
                        {plan.detection.conflicts.map((c) => conflictLabel(c.kind)).join(', ')}: will be neutralized (reversible)
                      </span>
                    </div>
                  )}
                </div>
              )}

              {/* Notices: single ⚠ group (detection warnings + operational notes) */}
              {plan && plan.notices.length > 0 && (
                <div style={styles.box}>
                  {plan.notices.map((notice, index) => (
                    <div key={index} style={styles.warn}>⚠ {notice}</div>
                  ))}
                </div>
              )}

              {/* Options */}
              {plan && (
                <div style={styles.box}>
                  <div style={styles.sectionTitle}>Options</div>
                  <div style={styles.row}>
                    <span style={styles.label}>AppID spoof</span>
                    <input
                      style={styles.input}
                      type="text"
                      inputMode="numeric"
                      pattern="[0-9]*"
                      value={request.spoofAppId}
                      onChange={(e) => set('spoofAppId', Number(e.target.value) || 480)}
                      disabled={enabled}
                    />
                  </div>
                  <div style={styles.row}>
                    <span style={styles.label}>ogAppID</span>
                    <input
                      style={styles.input}
                      type="text"
                      inputMode="numeric"
                      pattern="[0-9]*"
                      value={request.ogAppId}
                      onChange={(e) => set('ogAppId', Number(e.target.value) || 0)}
                      disabled={enabled}
                    />
                  </div>

                  {plan.detection.backends.photon !== 'none' && (
                    <div style={{ ...styles.box, marginTop: 8 }}>
                      <div style={styles.sectionTitle}>Photon ({plan.detection.backends.photon})</div>
                      {plan.detection.backends.photon === 'fusion' ? (
                        <div style={styles.row}>
                          <span style={styles.label}>Fusion App GUID</span>
                          <input style={styles.input} value={request.photon.fusionGuid} onChange={(e) => setPhoton('fusionGuid', e.target.value)} disabled={enabled} placeholder="app-id-xxxx" />
                        </div>
                      ) : (
                        <>
                          <div style={styles.row}>
                            <span style={styles.label}>Realtime App GUID</span>
                            <input style={styles.input} value={request.photon.realtimeGuid} onChange={(e) => setPhoton('realtimeGuid', e.target.value)} disabled={enabled} placeholder="app-id-xxxx" />
                          </div>
                          {plan.detection.backends.photonVoice && (
                            <div style={styles.row}>
                              <span style={styles.label}>Voice App GUID</span>
                              <input style={styles.input} value={request.photon.voiceGuid} onChange={(e) => setPhoton('voiceGuid', e.target.value)} disabled={enabled} placeholder="app-id-xxxx" />
                            </div>
                          )}
                        </>
                      )}
                    </div>
                  )}

                  {plan.detection.backends.playfab && (
                    <div style={{ ...styles.box, marginTop: 8 }}>
                      <div style={styles.sectionTitle}>PlayFab</div>
                      <div style={styles.row}>
                        <span style={styles.label}>TitleId (yours)</span>
                        <input style={styles.input} value={request.playfab.titleId} onChange={(e) => set('playfab', { ...request.playfab, titleId: e.target.value })} disabled={enabled} placeholder="XXXXX (empty = inert plugin)" />
                      </div>
                    </div>
                  )}

                  {plan.detection.backends.coherence && (
                    <div style={{ ...styles.box, marginTop: 8 }}>
                      <div style={styles.sectionTitle}>coherence</div>
                      <div style={styles.row}>
                        <span style={styles.label}>Runtime key</span>
                        <input style={styles.input} value={request.coherence.runtimeKey} onChange={(e) => set('coherence', { ...request.coherence, runtimeKey: e.target.value })} disabled={enabled || request.coherence.useShared} placeholder="your project (schema uploaded)" />
                      </div>
                      <div style={styles.checkboxRow}>
                        <span style={styles.checkboxDesc}>Use the SHARED community project (no account, availability not guaranteed)</span>
                        <input
                          type="checkbox"
                          checked={request.coherence.useShared}
                          onChange={(e) => set('coherence', { ...request.coherence, useShared: e.target.checked })}
                          disabled={enabled}
                        />
                      </div>
                    </div>
                  )}

                  {plan.detection.backends.eos && (
                    <div style={{ ...styles.box, marginTop: 8 }}>
                      <div style={styles.sectionTitle}>EOS (Epic Online Services)</div>
                      <div style={styles.checkboxRow}>
                        <span style={styles.checkboxDesc}>Deploy EOS_custom (your own Epic app, anonymous login)</span>
                        <input
                          type="checkbox"
                          checked={request.deployEosCustom}
                          onChange={(e) => set('deployEosCustom', e.target.checked)}
                          disabled={enabled}
                        />
                      </div>
                      {request.deployEosCustom && (
                        <div style={{ marginTop: 6 }}>
                          {(['productId', 'sandboxId', 'deploymentId', 'clientId', 'clientSecret'] as const).map((key) => (
                            <div style={styles.row} key={key}>
                              <span style={styles.label}>{key}</span>
                              <input style={styles.input} value={request.eos[key]} onChange={(e) => setEos(key, e.target.value)} disabled={enabled} placeholder={key === 'clientSecret' ? '...' : ''} />
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              )}

              {/* Result message */}
              {message && (
                <div style={{ ...styles.box, color: message.kind === 'error' ? '#e07b7b' : message.kind === 'success' ? '#6fdb8c' : '#d0d0d0' }}>
                  {message.text}
                </div>
              )}
            </>
          )}
        </div>

        <div style={styles.footer}>
          {enabled || broken ? (
            <>
              <button type="button" className="modal-btn" style={{ ...styles.footerBtn, ...styles.btnGhost }} onClick={onClose} disabled={busy}>
                Close
              </button>
              <button type="button" className="modal-btn" style={{ ...styles.footerBtn, ...styles.btnDanger }} onClick={handleDisable} disabled={busy}>
                {busy ? 'Disabling...' : 'Disable Online'}
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                className="modal-btn"
                style={{ ...styles.footerBtn, ...styles.btn, ...(blocked || busy ? styles.btnDisabled : {}) }}
                onClick={handleEnable}
                disabled={blocked || busy}
                title={blocked ? 'Resolve the missing prerequisites first' : undefined}
              >
                {busy ? 'Enabling...' : 'Enable'}
              </button>
              <button type="button" className="modal-btn" style={{ ...styles.footerBtn, ...styles.btnGhost }} onClick={handleReset} disabled={busy}>
                Reset
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
