import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SteamCheckStatus } from './types';
import type { SetPresenceDefaultModeArgs } from '../../types/online';
import { toErrorMessage } from '../../util/errors';

interface SettingsAetherSectionProps {
  enableWebviewDevtools: boolean;
  setEnableWebviewDevtools: (v: boolean) => void;
  enableTestUpdates: boolean;
  setEnableTestUpdates: (v: boolean) => void;
  useOstSource: boolean;
  setUseOstSource: (v: boolean) => void;
  ostWarningAcknowledged: boolean;
  setShowOstWarning: (v: boolean) => void;
  manifestRestoreOnStartup: boolean;
  setManifestRestoreOnStartup: (v: boolean) => void;
  presenceDefaultShowOnline: boolean;
  setPresenceDefaultShowOnline: (v: boolean) => void;
  customGameName: string;
  setCustomGameName: (v: string) => void;
  steamPath: string;
  setSteamPath: (v: string) => void;
  steamCheck: SteamCheckStatus;
  onBrowseSteamFolder: () => Promise<void>;
  onDetectSteamPath: () => Promise<void>;
  showStatus: (message: string, kind: 'success' | 'error') => void;
}

export const SettingsAetherSection: React.FC<SettingsAetherSectionProps> = ({
  enableWebviewDevtools,
  setEnableWebviewDevtools,
  enableTestUpdates,
  setEnableTestUpdates,
  useOstSource,
  setUseOstSource,
  ostWarningAcknowledged,
  setShowOstWarning,
  manifestRestoreOnStartup,
  setManifestRestoreOnStartup,
  presenceDefaultShowOnline,
  setPresenceDefaultShowOnline,
  customGameName,
  setCustomGameName,
  steamPath,
  setSteamPath,
  steamCheck,
  onBrowseSteamFolder,
  onDetectSteamPath,
  showStatus,
}) => {
  return (
    <div className="settings-group">
      <label className="settings-label">Aether</label>
      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Enable WebView devtools</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={enableWebviewDevtools}
            onChange={(e) => setEnableWebviewDevtools(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      <div className="settings-toggle-row" title="For beta testing, keep OFF unless the dev told you to enable it.">
        <span className="settings-toggle-text">Enable test updates</span>
        <label className="version-switch">
          <input type="checkbox" checked={enableTestUpdates} onChange={(e) => setEnableTestUpdates(e.target.checked)} />
          <span></span>
        </label>
      </div>

      <div className="settings-toggle-row" title="Keep it OFF unless the dev says otherwise.">
        <span className="settings-toggle-text">Use OST pattern source</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={useOstSource}
            onChange={async (e) => {
              const next = e.target.checked;
              if (next && !ostWarningAcknowledged) {
                setShowOstWarning(true);
                return;
              }
              const previous = !next;
              setUseOstSource(next);
              try {
                await invoke('set_ost_source_enabled', { enabled: next });
              } catch (err: unknown) {
                setUseOstSource(previous);
                showStatus(`Failed to set OST pattern source: ${toErrorMessage(err)}`, 'error');
              }
            }}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="AetherDLL copies backed-up .manifest missing from Steam\depotcache back into place on each Steam start, avoiding No Internet Connection Error when installing with its .manifest wiped out."
      >
        <span className="settings-toggle-text">Restore manifests on Steam startup</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={manifestRestoreOnStartup}
            onChange={async (e) => {
              const next = e.target.checked;
              const previous = !next;
              setManifestRestoreOnStartup(next);
              try {
                await invoke('set_manifest_restore_enabled', { enabled: next });
              } catch (err: unknown) {
                setManifestRestoreOnStartup(previous);
                showStatus(`Failed to set manifest restore: ${toErrorMessage(err)}`, 'error');
              }
            }}
          />
          <span></span>
        </label>
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
        <span className="settings-toggle-text">Clear AetherDesk caches</span>
        <button
          type="button"
          className="settings-small-btn"
          style={{
            width: '96px',
            minWidth: '96px',
            maxWidth: '96px',
            height: '33px',
            padding: '0',
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            boxSizing: 'border-box',
          }}
          onClick={async () => {
            try {
              const result: string = await invoke('clear_app_caches');
              try {
                Object.keys(localStorage)
                  .filter((key) => key.startsWith('aether_cover_') || key.startsWith('aether_hero_'))
                  .forEach((key) => localStorage.removeItem(key));
              } catch {
                /* localStorage unavailable: the backend part already succeeded */
              }
              showStatus(result, 'success');
            } catch (err: unknown) {
              showStatus(`Failed to clear caches: ${toErrorMessage(err)}`, 'error');
            }
          }}
        >
          Clear Cache
        </button>
      </div>
    </div>
  );
};
