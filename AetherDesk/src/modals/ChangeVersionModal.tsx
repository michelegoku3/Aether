import React, { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { requireSteamPath } from '../hooks/useSettings';
import { useModalDismiss } from '../hooks/useModalDismiss';
import { useGameBuilds, BuildInfo } from '../hooks/useGameBuilds';
import {
  ManualVersionEditor,
  LuaManifestRow,
  SpecificVersionGame,
} from './SpecificVersionModal';

export type VersionTab = 'manual' | 'auto' | 'builds';

export interface DepotManifestPin {
  depotId: number;
  manifestId: string;
}

export interface BuildPreview {
  buildId: number;
  date: string;
  title: string;
  pins: DepotManifestPin[];
  matchingPins: DepotManifestPin[];
  missingDepots: number[];
  unlistedDepots: number[];
  luaDepotCount: number;
}

export interface ApplyVersionReport {
  appliedPins: number;
  disabledDepots: number[];
  manifestsFound: number;
  manifestsMissing: string[];
  acfSyncedNow: boolean;
  acfQueued: boolean;
  luaBackupPath: string | null;
}

interface VersionProgressEvent {
  appId: number;
  buildId: number;
  step: number;
  message: string;
}

interface ChangeVersionModalProps {
  game: SpecificVersionGame;
  initialRows: LuaManifestRow[];
  initialTab?: VersionTab;
  onClose: () => void;
}

const TAB_LABELS: Record<VersionTab, string> = {
  manual: 'Manual',
  auto: 'Auto',
  builds: 'Builds',
};

/**
 * "Change Version" popup with three tabs:
 *  - Manual: the classic per-depot editor (unchanged behaviour);
 *  - Auto:   paste/enter a BuildID, preview the plan, apply it;
 *  - Builds: browse all published builds (or only the saved ones) and use
 *            any of them.
 */
const ChangeVersionModal = ({ game, initialRows, initialTab = 'manual', onClose }: ChangeVersionModalProps) => {
  const appId = Number(game.appId);
  const [activeTab, setActiveTab] = useState<VersionTab>(initialTab);
  // Build handed from the Builds tab to the Auto tab (lifted state, no events).
  const [requestedBuildId, setRequestedBuildId] = useState<string>('');

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal-container version-modal-container"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-header">
          <span className="modal-title">
            Change Version: <strong style={{ color: '#ffffff' }}>{game.name}</strong> ({game.appId})
          </span>
          <button onClick={onClose} className="modal-close-btn">&times;</button>
        </div>

        <div className="modal-separator"></div>

        <div className="version-tabs" role="tablist">
          {(Object.keys(TAB_LABELS) as VersionTab[]).map((tab) => (
            <button
              key={tab}
              role="tab"
              aria-selected={activeTab === tab}
              className={`version-tab${activeTab === tab ? ' active' : ''}`}
              onClick={() => setActiveTab(tab)}
            >
              {TAB_LABELS[tab]}
            </button>
          ))}
        </div>

        <div className="modal-body">
          {activeTab === 'manual' && (
            <ManualVersionEditor game={game} initialRows={initialRows} onClose={onClose} />
          )}

          {activeTab === 'auto' && (
            <AutoVersionTab
              appId={appId}
              initialBuildId={requestedBuildId}
              onClose={onClose}
            />
          )}

          {activeTab === 'builds' && (
            <BuildsVersionTab
              appId={appId}
              onUseBuild={(buildId) => {
                setRequestedBuildId(String(buildId));
                setActiveTab('auto');
              }}
            />
          )}
        </div>
      </div>
    </div>
  );
};

export default ChangeVersionModal;

/* ────────────────────────────────────────────────────────────────────────
 * Auto tab: BuildID input → preview → apply (with live progress)
 * ──────────────────────────────────────────────────────────────────────── */

interface AutoVersionTabProps {
  appId: number;
  initialBuildId: string;
  onClose: () => void;
}

const AutoVersionTab = ({ appId, initialBuildId, onClose }: AutoVersionTabProps) => {
  const [buildIdInput, setBuildIdInput] = useState(initialBuildId);
  const [preview, setPreview] = useState<BuildPreview | null>(null);
  const [report, setReport] = useState<ApplyVersionReport | null>(null);
  const [progress, setProgress] = useState<VersionProgressEvent | null>(null);
  const [status, setStatus] = useState<{ text: string; type: string }>({ text: '', type: 'info' });
  const [isPreviewing, setIsPreviewing] = useState(false);
  const [isApplying, setIsApplying] = useState(false);
  const watchdogRef = React.useRef<number | null>(null);

  useModalDismiss(onClose, isApplying || isPreviewing);

  const clearWatchdog = () => {
    if (watchdogRef.current !== null) {
      window.clearTimeout(watchdogRef.current);
      watchdogRef.current = null;
    }
  };

  // Clear any pending watchdog when the tab unmounts.
  React.useEffect(() => clearWatchdog, []);

  // When the Builds tab hands over a build id, look it up right away.
  React.useEffect(() => {
    if (initialBuildId) {
      void handlePreview(initialBuildId);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialBuildId]);

  const parseBuildId = (raw: string): number => {
    const digits = raw.replace(/[^0-9]/g, '');
    return digits ? Number(digits) : 0;
  };

  const handlePreview = async (raw?: string) => {
    if (isPreviewing) {
      return;
    }
    const buildId = parseBuildId(raw ?? buildIdInput);
    if (!buildId) {
      setStatus({ text: 'Enter a valid Build ID first.', type: 'error' });
      return;
    }
    setStatus({ text: '', type: 'info' });
    setReport(null);
    setPreview(null);
    setIsPreviewing(true);
    try {
      const steamPath = await requireSteamPath();
      const result = await invoke<BuildPreview>('get_build_preview', {
        appId,
        buildId,
        steamPath,
      });
      setBuildIdInput(String(result.buildId));
      setPreview(result);
    } catch (err: any) {
      setStatus({ text: String(err), type: 'error' });
    } finally {
      setIsPreviewing(false);
    }
  };

  const handleApply = async () => {
    if (!preview || isApplying) {
      return;
    }
    setIsApplying(true);
    setReport(null);
    setStatus({ text: '', type: 'info' });
    const buildId = preview.buildId;
    setProgress({ appId, buildId, step: 0, message: 'Starting...' });

    let unlisten: (() => void) | null = null;
    try {
      // Live progress is a nice-to-have: if the event listener cannot be
      // registered, the apply must still go through. Never let this step
      // leave the button spinning forever.
      try {
        unlisten = await listen<VersionProgressEvent>('versioning://progress', (event) => {
          if (event.payload.appId === appId && event.payload.buildId === buildId) {
            setProgress(event.payload);
          }
        });
      } catch {
        unlisten = null;
      }

      // Watchdog: the backend bounds its work (Depotbox lookup ≤ 45 s + a
      // fast local pipeline). If nothing settles by then, stop the spinner
      // and surface a message instead of loading forever. A late completion
      // still updates the UI when it arrives.
      watchdogRef.current = window.setTimeout(() => {
        setStatus({
          text: 'This is taking longer than expected. The operation continues in the background — check the Logs view for details.',
          type: 'error',
        });
        setProgress(null);
        setIsApplying(false);
      }, 90_000);

      const steamPath = await requireSteamPath();
      const result = await invoke<ApplyVersionReport>('apply_game_version', {
        appId,
        buildId,
        steamPath,
      });
      clearWatchdog();
      setReport(result);
      setProgress(null);
    } catch (err: any) {
      clearWatchdog();
      setStatus({ text: String(err), type: 'error' });
      setProgress(null);
    } finally {
      clearWatchdog();
      if (unlisten) {
        unlisten();
      }
      setIsApplying(false);
    }
  };

  return (
    <div className="version-tab-body">
      <div className="version-auto-row">
        <input
          className="version-build-input"
          type="text"
          inputMode="numeric"
          placeholder="Build ID (es. 24701871)"
          value={buildIdInput}
          disabled={isApplying || isPreviewing}
          onChange={(e) => setBuildIdInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !isApplying && !isPreviewing) {
              void handlePreview();
            }
          }}
        />
        <button
          className="panel-btn"
          onClick={() => void handlePreview()}
          disabled={isApplying || isPreviewing}
        >
          {isPreviewing ? 'Looking up...' : 'Preview'}
        </button>
      </div>

      {status.text && (
        <div className={`settings-alert ${status.type}`} style={{ padding: '10px 15px', fontSize: '12px' }}>
          {status.text}
        </div>
      )}

      {preview && !report && (
        <div className="version-preview">
          <div className="version-preview-header">
            {preview.title || `Build ${preview.buildId}`}
            {preview.date && <span className="version-preview-date">{preview.date}</span>}
          </div>
          <ul className="version-preview-facts">
            <li><strong>{preview.matchingPins.length}</strong> manifest pin(s) will be applied</li>
            {preview.missingDepots.length > 0 && (
              <li><strong>{preview.missingDepots.length}</strong> depot(s) will be disabled (not in this build)</li>
            )}
            {preview.unlistedDepots.length > 0 && (
              <li>{preview.unlistedDepots.length} depot(s) of this build are not in the game's Lua</li>
            )}
            {preview.matchingPins.length === 0 && (
              <li className="version-preview-warn">No depot of this build matches the game's Lua.</li>
            )}
          </ul>
          <div className="version-auto-row">
            <button
              className="panel-btn"
              onClick={() => void handleApply()}
              disabled={isApplying || preview.matchingPins.length === 0}
            >
              {isApplying ? 'Applying...' : 'Apply Build'}
            </button>
          </div>
        </div>
      )}

      {progress && (
        <div className="version-progress">
          <div className="version-progress-bar">
            <div className="version-progress-fill" style={{ width: `${progress.step}%` }} />
          </div>
          <div className="version-progress-label">{progress.message}</div>
        </div>
      )}

      {report && (
        <div className="version-report">
          <div className="settings-alert success" style={{ padding: '10px 15px', fontSize: '12px' }}>
            Build {preview?.buildId ?? ''} applied: {report.appliedPins} manifest pin(s) written.
            {report.disabledDepots.length > 0 && (
              <> {report.disabledDepots.length} depot(s) disabled because they do not exist in this build.</>
            )}
          </div>
          {report.acfSyncedNow && (
            <div className="version-report-note">
              Steam ACF updated — Steam will download this build.
            </div>
          )}
          {report.acfQueued && (
            <div className="version-report-note">
              The ACF edit is queued and will be applied automatically once Steam releases it
              (e.g. after the game finishes downloading).
            </div>
          )}
          {report.manifestsMissing.length > 0 && (
            <div className="version-report-note">
              {report.manifestsMissing.length} pinned manifest file(s) are not in the depotcache yet.
              Steam downloads them on demand from the pinned manifests.
            </div>
          )}
          <div className="version-auto-row">
            <button className="panel-btn" onClick={onClose}>Done</button>
          </div>
        </div>
      )}
    </div>
  );
};

/* ────────────────────────────────────────────────────────────────────────
 * Builds tab: browse all published builds / saved bookmarks
 * ──────────────────────────────────────────────────────────────────────── */

interface BuildsVersionTabProps {
  appId: number;
  onUseBuild: (buildId: number) => void;
}

const BuildsVersionTab = ({ appId, onUseBuild }: BuildsVersionTabProps) => {
  const { builds, savedIds, loading, error, reload, toggleSaved } = useGameBuilds(appId);
  const [onlySaved, setOnlySaved] = useState(false);
  const [savingId, setSavingId] = useState<number | null>(null);

  const visibleBuilds = useMemo(() => {
    if (!onlySaved) {
      return builds;
    }
    return builds.filter((build) => savedIds.has(build.buildId));
  }, [builds, savedIds, onlySaved]);

  const handleToggleSaved = async (build: BuildInfo) => {
    setSavingId(build.buildId);
    try {
      await toggleSaved(build);
    } finally {
      setSavingId(null);
    }
  };

  return (
    <div className="version-tab-body">
      <div className="version-builds-toolbar">
        <div className="version-filter-chips">
          <button
            className={`version-chip${!onlySaved ? ' active' : ''}`}
            onClick={() => setOnlySaved(false)}
          >
            All
          </button>
          <button
            className={`version-chip${onlySaved ? ' active' : ''}`}
            onClick={() => setOnlySaved(true)}
          >
            Saved ({savedIds.size})
          </button>
        </div>
        <button className="version-refresh-btn" onClick={() => void reload()} title="Refresh builds">
          ⟳
        </button>
      </div>

      {loading && <div className="version-builds-empty">Loading builds…</div>}

      {!loading && error && (
        <div className="settings-alert error" style={{ padding: '10px 15px', fontSize: '12px' }}>
          {error}
        </div>
      )}

      {!loading && !error && visibleBuilds.length === 0 && (
        <div className="version-builds-empty">
          {onlySaved ? 'No saved builds yet — star one from the All list.' : 'No builds found.'}
        </div>
      )}

      {!loading && !error && visibleBuilds.length > 0 && (
        <div className="version-build-list">
          {visibleBuilds.map((build) => (
            <div className="version-build-row" key={build.buildId}>
              <button
                className={`version-build-star${savedIds.has(build.buildId) ? ' saved' : ''}`}
                title={savedIds.has(build.buildId) ? 'Remove from saved builds' : 'Save this build'}
                onClick={() => void handleToggleSaved(build)}
                disabled={savingId === build.buildId}
              >
                {savedIds.has(build.buildId) ? '★' : '☆'}
              </button>
              <div className="version-build-main">
                <div className="version-build-title">{build.title || `Build ${build.buildId}`}</div>
                <div className="version-build-meta">
                  {build.date && <span className="version-build-date">{build.date}</span>}
                  <span className="version-build-bid">Build #{build.buildId}</span>
                </div>
              </div>
              <button
                className="panel-btn version-build-use"
                onClick={() => onUseBuild(build.buildId)}
              >
                Use
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
