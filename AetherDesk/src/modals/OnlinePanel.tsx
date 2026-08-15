import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { LibraryActionGame } from './LibraryGameActionsModal';

// ---------------------------------------------------------------------------
// Tipi specchio dei comandi Rust (serde rename_all = camelCase)
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
  steamStubPatch: boolean;
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
  suggestions: string[];
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
  steamStubPatch: boolean;
  photon: { realtimeGuid: string; voiceGuid: string; fusionGuid: string };
  eos: { productId: string; sandboxId: string; deploymentId: string; clientId: string; clientSecret: string };
  playfab: { titleId: string };
  coherence: { runtimeKey: string; useShared: boolean };
  deployEosCustom: boolean;
}

// ---------------------------------------------------------------------------
// Stili (contenuti nel componente: nessun impatto sul resto dell'app)
// ---------------------------------------------------------------------------

const styles = {
  body: { padding: '16px 20px', overflowY: 'auto' as const, maxHeight: '62vh' },
  section: { marginBottom: '14px' },
  sectionTitle: { fontSize: '12px', fontWeight: 700, textTransform: 'uppercase' as const, letterSpacing: '0.06em', opacity: 0.65, marginBottom: '8px' },
  row: { display: 'flex' as const, gap: '10px', alignItems: 'center', marginBottom: '8px', flexWrap: 'wrap' as const },
  label: { fontSize: '12px', opacity: 0.75, minWidth: '150px' },
  input: { background: '#1b1b1f', border: '1px solid #2c2c31', borderRadius: '6px', color: '#eee', padding: '6px 8px', fontSize: '13px', flex: 1, minWidth: '160px' },
  chip: { padding: '2px 10px', borderRadius: '999px', fontSize: '12px', fontWeight: 600 },
  chipEnabled: { background: '#16351f', color: '#6fdb8c', border: '1px solid #2c6e42' },
  chipOff: { background: '#2a2a2e', color: '#9a9aa0', border: '1px solid #3a3a40' },
  chipBroken: { background: '#3a1d1d', color: '#e07b7b', border: '1px solid #6e2c2c' },
  box: { background: '#17171b', border: '1px solid #232329', borderRadius: '8px', padding: '10px 12px', marginBottom: '8px', fontSize: '12px' },
  warn: { color: '#e0b06a' },
  error: { color: '#e07b7b' },
  ok: { color: '#6fdb8c' },
  muted: { opacity: 0.6 },
  footer: { display: 'flex' as const, gap: '10px', justifyContent: 'flex-end' as const, padding: '12px 20px', borderTop: '1px solid #232329' },
  btn: { background: '#2c6e42', color: '#fff', border: 'none', borderRadius: '8px', padding: '8px 16px', fontSize: '13px', fontWeight: 600, cursor: 'pointer' },
  btnDisabled: { opacity: 0.45, cursor: 'not-allowed' },
  btnGhost: { background: '#232329', color: '#ddd', border: '1px solid #2f2f35', borderRadius: '8px', padding: '8px 14px', fontSize: '13px', cursor: 'pointer' },
  btnDanger: { background: '#5a2323', color: '#ffd9d9', border: 'none', borderRadius: '8px', padding: '8px 14px', fontSize: '13px', cursor: 'pointer' },
  header: { display: 'flex' as const, justifyContent: 'space-between' as const, alignItems: 'center' as const, padding: '14px 20px', borderBottom: '1px solid #232329' },
  headerTitle: { margin: 0, fontSize: '16px', fontWeight: 700 },
  close: { background: 'none', border: 'none', color: '#999', fontSize: '20px', cursor: 'pointer', lineHeight: 1 },
};

const emptyRequest = (ogAppId: number): OnlineEnableRequest => ({
  ogAppId,
  spoofAppId: 480,
  steamStubPatch: false,
  photon: { realtimeGuid: '', voiceGuid: '', fusionGuid: '' },
  eos: { productId: '', sandboxId: '', deploymentId: '', clientId: '', clientSecret: '' },
  playfab: { titleId: '' },
  coherence: { runtimeKey: '', useShared: false },
  deployEosCustom: false,
});

const backendChips = (backends: OnlineBackendReport): string[] => {
  const chips: string[] = [];
  if (backends.photon !== 'none') {
    chips.push(`Photon ${backends.photon}${backends.photonVoice ? ' + Voice' : ''}`);
  }
  if (backends.eos) chips.push('EOS');
  if (backends.playfab) chips.push('PlayFab');
  if (backends.coherence) chips.push('coherence');
  if (chips.length === 0) chips.push('Steam P2P (nessun backend)');
  return chips;
};

const conflictLabel = (kind: string): string => {
  switch (kind) {
    case 'coldClientLoader': return 'ColdClientLoader';
    case 'steamFix': return 'SteamFix';
    case 'onlineFix': return 'OnlineFix';
    case 'namedFixFile': return 'File di fix (winmm/…)';
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
      // Precompila il default EOS dal rilevamento (UCO2 è il fix preferito).
      setRequest((prev) => ({
        ...prev,
        deployEosCustom: planResult.detection.backends.eos ? prev.deployEosCustom || true : prev.deployEosCustom,
        ogAppId: prev.ogAppId || Number(game.appId) || 0,
      }));
    } catch (err) {
      setMessage({ text: `Piano non disponibile: ${err}`, kind: 'error' });
    } finally {
      setLoading(false);
    }
  }, [game.appId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleEnable = async () => {
    setBusy(true);
    setMessage({ text: 'Attivazione in corso…', kind: 'info' });
    try {
      const result = await invoke<OnlineActionResult>('enable_online', {
        appId: Number(game.appId),
        request,
      });
      setMessage({ text: result.message, kind: result.success ? 'success' : 'error' });
      await refresh();
    } catch (err) {
      setMessage({ text: `Attivazione fallita: ${err}`, kind: 'error' });
    } finally {
      setBusy(false);
    }
  };

  const handleDisable = async () => {
    setBusy(true);
    setMessage({ text: 'Disattivazione in corso…', kind: 'info' });
    try {
      const result = await invoke<OnlineActionResult>('disable_online', { appId: Number(game.appId) });
      setMessage({ text: result.message, kind: result.success ? 'success' : 'error' });
      await refresh();
    } catch (err) {
      setMessage({ text: `Disattivazione fallita: ${err}`, kind: 'error' });
    } finally {
      setBusy(false);
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

  return (
    <div className="modal-overlay" onClick={busy ? undefined : onClose}>
      <div
        className="modal-container"
        style={{ width: 720, maxHeight: '86vh' }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={styles.header}>
          <h3 style={styles.headerTitle}>Enable Online — {game.name}</h3>
          <button type="button" style={styles.close} onClick={onClose} disabled={busy} aria-label="Close">×</button>
        </div>

        <div style={styles.body}>
          {loading ? (
            <div style={styles.muted}>Analisi del gioco in corso…</div>
          ) : (
            <>
              {/* Stato */}
              <div style={styles.row}>
                {enabled && <span style={{ ...styles.chip, ...styles.chipEnabled }}>● Online attivo</span>}
                {broken && <span style={{ ...styles.chip, ...styles.chipBroken }}>● Configurazione mancante</span>}
                {!enabled && !broken && <span style={{ ...styles.chip, ...styles.chipOff }}>○ Non configurato</span>}
                {status?.record?.bundleVersion && <span style={styles.muted}>bundle {status.record.bundleVersion}</span>}
              </div>

              {/* Prerequisiti */}
              {plan && plan.prerequisites.errors.length > 0 && (
                <div style={styles.box}>
                  {plan.prerequisites.errors.map((error, index) => (
                    <div key={index} style={styles.error}>⚠ {error}</div>
                  ))}
                </div>
              )}

              {/* Rilevamento */}
              {plan && (
                <div style={styles.box}>
                  <div style={styles.row}>
                    <span style={styles.label}>Bundle UCOnline2</span>
                    <span style={plan.prerequisites.bundleOk ? styles.ok : styles.error}>
                      {plan.prerequisites.bundleOk
                        ? `✓ presente${plan.prerequisites.bundleVersion ? ` (${plan.prerequisites.bundleVersion})` : ''}`
                        : '✗ mancante'}
                    </span>
                  </div>
                  <div style={styles.row}>
                    <span style={styles.label}>Cartella gioco scrivibile</span>
                    <span style={plan.prerequisites.steamApiDirWritable ? styles.ok : styles.error}>
                      {plan.prerequisites.steamApiDirWritable ? '✓ sì' : '✗ no'}
                    </span>
                  </div>
                  <div style={styles.row}>
                    <span style={styles.label}>Engine</span>
                    <span>{plan.detection.engine} · {plan.detection.arch === 'x64' ? '64-bit' : '32-bit'}</span>
                  </div>
                  {plan.detection.gameExe && (
                    <div style={styles.row}>
                      <span style={styles.label}>Eseguibile</span>
                      <span style={styles.muted}>{plan.detection.gameExe}</span>
                    </div>
                  )}
                  {plan.detection.steamApiDir && (
                    <div style={styles.row}>
                      <span style={styles.label}>Posizione DLL</span>
                      <span style={styles.muted}>{plan.detection.steamApiDir}</span>
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
                      <span style={styles.label}>Conflitti</span>
                      <span style={styles.warn}>
                        {plan.detection.conflicts.map((c) => conflictLabel(c.kind)).join(', ')} — verranno neutralizzati (reversibile)
                      </span>
                    </div>
                  )}
                  {plan.detection.warnings.map((warning, index) => (
                    <div key={index} style={styles.warn}>⚠ {warning}</div>
                  ))}
                </div>
              )}

              {/* Suggerimenti */}
              {plan && plan.suggestions.length > 0 && (
                <div style={styles.box}>
                  {plan.suggestions.map((suggestion, index) => (
                    <div key={index} style={styles.muted}>💡 {suggestion}</div>
                  ))}
                </div>
              )}

              {/* Opzioni */}
              <div style={styles.section}>
                <div style={styles.sectionTitle}>Opzioni</div>
                <div style={styles.row}>
                  <span style={styles.label}>AppId spoofato (Spacewar)</span>
                  <input
                    style={styles.input}
                    type="number"
                    value={request.spoofAppId}
                    onChange={(e) => set('spoofAppId', Number(e.target.value) || 480)}
                    disabled={enabled}
                  />
                </div>
                <div style={styles.row}>
                  <span style={styles.label}>ogAppId (reale)</span>
                  <input
                    style={styles.input}
                    type="number"
                    value={request.ogAppId}
                    onChange={(e) => set('ogAppId', Number(e.target.value) || 0)}
                    disabled={enabled}
                  />
                </div>
                <div style={styles.row}>
                  <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13 }}>
                    <input
                      type="checkbox"
                      checked={request.steamStubPatch}
                      onChange={(e) => set('steamStubPatch', e.target.checked)}
                      disabled={enabled}
                    />
                    Patch SteamStub a runtime (GetStubbedLol) — per giochi che Steamless non riesce a sistemare
                  </label>
                </div>

                {plan && plan.detection.backends.photon !== 'none' && (
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

                {plan && plan.detection.backends.playfab && (
                  <div style={{ ...styles.box, marginTop: 8 }}>
                    <div style={styles.sectionTitle}>PlayFab</div>
                    <div style={styles.row}>
                      <span style={styles.label}>TitleId (tuo)</span>
                      <input style={styles.input} value={request.playfab.titleId} onChange={(e) => set('playfab', { ...request.playfab, titleId: e.target.value })} disabled={enabled} placeholder="XXXXX (vuoto = plugin inerte)" />
                    </div>
                  </div>
                )}

                {plan && plan.detection.backends.coherence && (
                  <div style={{ ...styles.box, marginTop: 8 }}>
                    <div style={styles.sectionTitle}>coherence</div>
                    <div style={styles.row}>
                      <span style={styles.label}>Runtime key</span>
                      <input style={styles.input} value={request.coherence.runtimeKey} onChange={(e) => set('coherence', { ...request.coherence, runtimeKey: e.target.value })} disabled={enabled || request.coherence.useShared} placeholder="progetto tuo (schema caricato)" />
                    </div>
                    <div style={styles.row}>
                      <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13 }}>
                        <input
                          type="checkbox"
                          checked={request.coherence.useShared}
                          onChange={(e) => set('coherence', { ...request.coherence, useShared: e.target.checked })}
                          disabled={enabled}
                        />
                        Usa il progetto SHARED community (nessun account, disponibilità non garantita)
                      </label>
                    </div>
                  </div>
                )}

                {plan && plan.detection.backends.eos && (
                  <div style={{ ...styles.box, marginTop: 8 }}>
                    <div style={styles.sectionTitle}>EOS (Epic Online Services)</div>
                    <div style={styles.row}>
                      <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13 }}>
                        <input
                          type="checkbox"
                          checked={request.deployEosCustom}
                          onChange={(e) => set('deployEosCustom', e.target.checked)}
                          disabled={enabled}
                        />
                        Deploya EOS_custom (app Epic tua, login anonimo)
                      </label>
                    </div>
                    {request.deployEosCustom && (
                      <div style={{ marginTop: 6 }}>
                        {(['productId', 'sandboxId', 'deploymentId', 'clientId', 'clientSecret'] as const).map((key) => (
                          <div style={styles.row} key={key}>
                            <span style={styles.label}>{key}</span>
                            <input style={styles.input} value={request.eos[key]} onChange={(e) => setEos(key, e.target.value)} disabled={enabled} placeholder={key === 'clientSecret' ? '…' : ''} />
                          </div>
                        ))}
                      </div>
                    )}
                    <div style={styles.muted}>
                      Nota: se il gioco è gestito anche dall'onlinefix Aether, tieni UNA sola via attiva per EOS (doppio hook su EOSSDK può causare instabilità).
                    </div>
                  </div>
                )}
              </div>

              {/* Messaggio esito */}
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
              <button type="button" style={styles.btnGhost} onClick={onClose} disabled={busy}>Chiudi</button>
              <button type="button" style={styles.btnDanger} onClick={handleDisable} disabled={busy}>
                {busy ? 'Disattivazione…' : 'Disattiva Online'}
              </button>
            </>
          ) : (
            <>
              <button type="button" style={styles.btnGhost} onClick={onClose} disabled={busy}>Annulla</button>
              <button
                type="button"
                style={{ ...styles.btn, ...(blocked || busy ? styles.btnDisabled : {}) }}
                onClick={handleEnable}
                disabled={blocked || busy}
                title={blocked ? 'Risolvi i prerequisiti mancanti' : undefined}
              >
                {busy ? 'Attivazione…' : 'Attiva Online'}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
