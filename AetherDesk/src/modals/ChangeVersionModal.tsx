import React, { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { requireSteamPath } from '../hooks/useSettings';
import { useModalDismiss } from '../hooks/useModalDismiss';
import { useGameBuilds, BuildInfo } from '../hooks/useGameBuilds';
import { useWatchdog } from '../hooks/useWatchdog';
import { RefreshIcon } from '../ui/icons';
import { StatusAlert } from '../ui/StatusAlert';
import { emptyStatus, StatusMessage } from '../types/ui';
import { formatDateDDMMYYYY } from '../util/dates';
import {
  ManualVersionEditor,
  LuaManifestRow,
  SpecificVersionGame,
} from './SpecificVersionModal';

export type VersionTab = 'manual' | 'auto';

export interface ApplyVersionReport {
  appliedPins: number;
  disabledDepots: number[];
  manifestsFound: number;
  manifestsMissing: string[];
  acfSyncedNow: boolean;
  acfQueued: boolean;
  luaBackupPath: string | null;
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
};

/**
 * "Change Version" popup with two tabs:
 *  - Manual: the classic per-depot editor (unchanged behaviour);
 *  - Auto:   browse builds, filter favourites, enter a BuildID, apply it.
 */
const ChangeVersionModal = ({ game, initialRows, initialTab = 'manual', onClose }: ChangeVersionModalProps) => {
  const appId = Number(game.appId);
  const [activeTab, setActiveTab] = useState<VersionTab>(initialTab);

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

        <div className="modal-body version-modal-body">
          {activeTab === 'manual' && (
            <ManualVersionEditor game={game} initialRows={initialRows} onClose={onClose} />
          )}

          {activeTab === 'auto' && (
            <AutoBuildsTab
              appId={appId}
              onClose={onClose}
            />
          )}
        </div>
      </div>
    </div>
  );
};

export default ChangeVersionModal;

/* ────────────────────────────────────────────────────────────────────────
 * Auto+Builds tab: text input + star filter + refresh + builds list
 * + Open SteamDB / Apply Build buttons
 * ──────────────────────────────────────────────────────────────────────── */

interface AutoBuildsTabProps {
  appId: number;
  onClose: () => void;
}

const AutoBuildsTab = ({ appId, onClose }: AutoBuildsTabProps) => {
  const [buildIdInput, setBuildIdInput] = useState('');
  const [report, setReport] = useState<ApplyVersionReport | null>(null);
  const [appliedBuildId, setAppliedBuildId] = useState(0);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());
  const [isApplying, setIsApplying] = useState(false);
  const { builds, savedIds, loading, error, reload, toggleSaved } = useGameBuilds(appId);
  const [onlySaved, setOnlySaved] = useState(false);
  const [savingId, setSavingId] = useState<number | null>(null);
  const { arm: armWatchdog, clear: clearWatchdog } = useWatchdog();

  useModalDismiss(onClose, isApplying);

  // The success toast is transient: give the user a moment to read it, then
  // dismiss it on its own (same behaviour as the other green status alerts).
  React.useEffect(() => {
    if (!report) return;
    const timer = window.setTimeout(() => {
      setReport(null);
      setAppliedBuildId(0);
    }, 5000);
    return () => window.clearTimeout(timer);
  }, [report]);

  const visibleBuilds = useMemo(() => {
    if (!onlySaved) return builds;
    return builds.filter((build) => savedIds.has(build.buildId));
  }, [builds, savedIds, onlySaved]);

  const parseBuildId = (raw: string): number => {
    const digits = raw.replace(/[^0-9]/g, '');
    return digits ? Number(digits) : 0;
  };

  const handleApply = async () => {
    if (isApplying) return;
    const buildId = parseBuildId(buildIdInput);
    if (!buildId) {
      setStatus({ text: 'Enter a valid Build ID first.', type: 'error' });
      return;
    }
    setIsApplying(true);
    setReport(null);
    setStatus({ text: '', type: 'info' });

    try {
      armWatchdog(() => {
        setStatus({
          text: 'This is taking longer than expected. The operation continues in the background — check the Logs view for details.',
          type: 'error',
        });
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
      setAppliedBuildId(buildId);
    } catch (err: any) {
      clearWatchdog();
      setStatus({ text: String(err), type: 'error' });
    } finally {
      clearWatchdog();
      setIsApplying(false);
    }
  };

  const handleOpenSteamDb = async () => {
    try {
      // Auto flow is build-driven: open the patchnotes page (where each
      // published build is documented), not the per-depot page used by the
      // Manual editor.
      await invoke('open_steamdb_patchnotes', { appId });
    } catch (err: any) {
      setStatus({ text: `Failed to open SteamDB: ${err}`, type: 'error' });
    }
  };

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
      {/* Input row: Build ID input (with clear ×) + star filter + refresh */}
      <div className="version-auto-input-row">
        <div className="version-build-input-wrap">
          <input
            className="version-build-input"
            type="text"
            inputMode="numeric"
            placeholder="Build ID (es. 24701871)"
            value={buildIdInput}
            disabled={isApplying}
            onChange={(e) => setBuildIdInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !isApplying) {
                void handleApply();
              }
            }}
          />
          {buildIdInput.length > 0 && (
            <button
              type="button"
              className="version-build-clear"
              title="Clear Build ID"
              aria-label="Clear Build ID"
              onClick={() => setBuildIdInput('')}
            >
              &times;
            </button>
          )}
        </div>
        <button
          className={`version-star-filter-btn${onlySaved ? ' active' : ''}`}
          title={onlySaved ? 'Show all builds' : 'Show saved builds'}
          onClick={() => setOnlySaved(!onlySaved)}
          disabled={isApplying}
        >
          {onlySaved ? '★' : '☆'}
        </button>
        <button
          className="version-refresh-btn"
          title="Refresh builds"
          onClick={() => void reload()}
          disabled={isApplying}
        >
          <RefreshIcon size={15} />
        </button>
      </div>

      <StatusAlert status={status} className="settings-alert--compact" />

      {/* Builds list */}
      {loading && <div className="version-builds-empty">Loading builds…</div>}

      {!loading && error && (
        <StatusAlert status={{ text: error, type: 'error' }} className="settings-alert--compact" />
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
                  {build.date && <span className="version-build-date">{formatDateDDMMYYYY(build.date)}</span>}
                  <span className="version-build-bid">Build #{build.buildId}</span>
                </div>
              </div>
              <button
                className="panel-btn version-build-use"
                disabled={isApplying || report !== null}
                onClick={() => {
                  setBuildIdInput(String(build.buildId));
                }}
              >
                Use
              </button>
            </div>
          ))}
        </div>
      )}

      {report && (
        <div className="version-report">
          <StatusAlert
            status={{
              text: `Build ${appliedBuildId} applied: ${report.appliedPins} manifest pin(s) written.${
                report.disabledDepots.length > 0
                  ? ` ${report.disabledDepots.length} depot(s) disabled because they do not exist in this build.`
                  : ''
              }`,
              type: 'success',
            }}
            className="settings-alert--compact"
          />
        </div>
      )}

      {/* Action buttons */}
      <div className="version-actions">
        <button
          className="panel-btn"
          onClick={handleOpenSteamDb}
          disabled={isApplying}
        >
          Open SteamDB
        </button>
        <button
          className="panel-btn"
          onClick={() => void handleApply()}
          disabled={isApplying || !parseBuildId(buildIdInput)}
        >
          {isApplying ? 'Applying...' : 'Apply Build'}
        </button>
      </div>
    </div>
  );
};
