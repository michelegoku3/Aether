import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SteamCheckStatus } from './types';
import type { SetPresenceDefaultModeArgs } from '../../types/online';
import { toErrorMessage } from '../../util/errors';
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
  return (
    <div className="settings-group">
      <label className="settings-label">Steam</label>
      <div className="settings-toggle-row settings-field-column">
        <span className="settings-toggle-text">Steam Installation Path</span>
        <p className="settings-desc">
          The main directory path where Steam is installed on your PC, required for configuration.
        </p>
        <input
          type="text"
          placeholder="C:\Program Files (x86)\Steam"
          value={steamPath}
          onChange={(e) => setSteamPath(e.target.value)}
          className="settings-input"
        />
        {steamCheck.state !== 'idle' && (
          <p className={`settings-desc steam-path-status steam-path-${steamCheck.state}`}>{steamCheck.message}</p>
        )}
      </div>

      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Browse for the Steam folder</span>
        <button
          type="button"
          className="settings-small-btn"
          style={{
            minWidth: '96px',
            height: '33px',
            padding: '0 12px',
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            boxSizing: 'border-box',
          }}
          onClick={() => void onBrowseSteamFolder()}
        >
          Browse
        </button>
      </div>

      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Detect Steam automatically</span>
        <button
          type="button"
          className="settings-small-btn"
          style={{
            minWidth: '96px',
            height: '33px',
            padding: '0 12px',
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            boxSizing: 'border-box',
          }}
          onClick={() => void onDetectSteamPath()}
        >
          Auto Detect
        </button>
      </div>

      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Show every game to friends by default</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={presenceDefaultShowOnline}
            onChange={async (e) => {
              const next = e.target.checked;
              const previous = !next;
              setPresenceDefaultShowOnline(next);
              try {
                // IPC key `showOnline` (Rust `show_online` → lowerCamelCase),
                // typed in `types/online.ts`. NOT the TOML value, which
                // stays "showonline": see docs/shared_contracts.md §7.
                const args: SetPresenceDefaultModeArgs = { showOnline: next };
                await invoke<string>('set_presence_default_mode', args);
              } catch (err: unknown) {
                setPresenceDefaultShowOnline(previous);
                showStatus(`Failed to set presence default mode: ${toErrorMessage(err)}`, 'error');
              }
            }}
          />
          <span></span>
        </label>
      </div>

      {presenceDefaultShowOnline && (
        <div className="settings-toggle-row settings-field-column">
          <span className="settings-toggle-text">Custom game display name</span>
          <p className="settings-desc">
            Overrides the game name shown to friends on Steam, leave empty to show the real title.
          </p>
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
