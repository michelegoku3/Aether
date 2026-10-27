import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SteamCheckStatus } from './types';
import type { AppInfoRepair } from '../../types/steam';
import { toErrorMessage } from '../../util/errors';
import { FolderIcon } from '../../ui/icons';
import { SettingsLibrarySection } from './SettingsLibrarySection';
import type { SteamAccount } from './libraryBanner';

interface SettingsSteamSectionProps {
  steamPath: string;
  setSteamPath: (value: string) => void;
  steamCheck: SteamCheckStatus;
  onBrowseSteamFolder: () => Promise<void>;
  onDetectSteamPath: () => Promise<void>;
  presenceDefaultShowOnline: boolean;
  setPresenceDefaultShowOnline: (value: boolean) => void;
  customGameName: string;
  setCustomGameName: (value: string) => void;
  libraryEnabled: boolean;
  setLibraryEnabled: (value: boolean) => void;
  libraryAppIds: string;
  setLibraryAppIds: (value: string) => void;
  librarySteamId: string;
  setLibrarySteamId: (value: string) => void;
  steamAccounts: SteamAccount[];
  steamAccountsLoaded: boolean;
  steamAccountsError: boolean;
  showStatus: (message: string, kind: 'success' | 'error') => void;
}

/**
 * Steam-facing settings: the client-adjacent options, kept together and apart
 * from the Aether-specific switches.
 */
export const SettingsSteamSection: React.FC<SettingsSteamSectionProps> = ({
  steamPath,
  setSteamPath,
  steamCheck,
  onBrowseSteamFolder,
  onDetectSteamPath,
  presenceDefaultShowOnline,
  setPresenceDefaultShowOnline,
  customGameName,
  setCustomGameName,
  libraryEnabled,
  setLibraryEnabled,
  libraryAppIds,
  setLibraryAppIds,
  librarySteamId,
  setLibrarySteamId,
  steamAccounts,
  steamAccountsLoaded,
  steamAccountsError,
  showStatus,
}) => {
  // Steam owns appinfo.vdf while it runs and rewrites it on exit, so the
  // rename only makes sense with the client closed; the backend refuses
  // otherwise, and this asks first.
  const repairAppInfoCache = async () => {
    if (!window.confirm('Close Steam before repairing its metadata cache. Continue?')) {
      return;
    }
    try {
      const result = await invoke<AppInfoRepair>('repair_appinfo_cache');
      showStatus(
        `Steam metadata cache moved to ${result.backupPath} (${result.bytes} bytes). Steam rebuilds it on the next start.`,
        'success'
      );
    } catch (err: unknown) {
      showStatus(`Failed to repair the Steam metadata cache: ${toErrorMessage(err)}`, 'error');
    }
  };

  return (
    <div className="settings-group">
      <label className="settings-label">Steam</label>
      <div className="settings-toggle-row settings-field-column">
        <span className="settings-toggle-text">Steam Installation Path</span>
        <div className="settings-input-wrap">
          <input
            type="text"
            placeholder="C:\Program Files (x86)\Steam"
            value={steamPath}
            onChange={(e) => setSteamPath(e.target.value)}
            className="settings-input"
          />
          <button
            type="button"
            className="settings-input-eye"
            title="Browse for the Steam folder"
            aria-label="Browse for the Steam folder"
            onClick={() => void onBrowseSteamFolder()}
          >
            <FolderIcon size={16} />
          </button>
        </div>
        {steamCheck.state !== 'idle' && (
          <p className={`settings-desc steam-path-status steam-path-${steamCheck.state}`}>{steamCheck.message}</p>
        )}
      </div>

      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Detect Steam automatically</span>
        <button
          type="button"
          className="settings-small-btn settings-action-btn"
          onClick={() => void onDetectSteamPath()}
        >
          Auto Detect
        </button>
      </div>

      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Repair Steam metadata cache</span>
        <button
          type="button"
          className="settings-small-btn settings-action-btn"
          title="Close Steam first: the cache is rebuilt on the next start and the old file is kept as a backup."
          onClick={() => void repairAppInfoCache()}
        >
          Repair Cache
        </button>
      </div>

      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Show every game you are playing to friends</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={presenceDefaultShowOnline}
            onChange={(e) => setPresenceDefaultShowOnline(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      {presenceDefaultShowOnline && (
        <div className="settings-toggle-row settings-field-column">
          <span className="settings-toggle-text">Custom game display name</span>
          <input
            type="text"
            className="settings-input"
            placeholder="Enter custom name (e.g. Aether)"
            value={customGameName}
            onChange={(e) => setCustomGameName(e.target.value)}
          />
        </div>
      )}

      <SettingsLibrarySection
        enabled={libraryEnabled}
        setEnabled={setLibraryEnabled}
        appIds={libraryAppIds}
        setAppIds={setLibraryAppIds}
        steamId={librarySteamId}
        setSteamId={setLibrarySteamId}
        accounts={steamAccounts}
        accountsLoaded={steamAccountsLoaded}
        accountsError={steamAccountsError}
      />
    </div>
  );
};
