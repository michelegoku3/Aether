import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SteamCheckStatus } from './types';

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
      <div
        className="settings-toggle-row"
        title="Open WebView developer tools. They open only when you switch this ON manually."
      >
        <span className="settings-toggle-text">Enable WebView devtools</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={enableWebviewDevtools}
            onChange={async (e) => {
              const next = e.target.checked;
              setEnableWebviewDevtools(next);
              if (next) {
                try {
                  await invoke('open_webview_devtools');
                } catch (err) {
                  console.warn('Unable to open WebView devtools:', err);
                }
              }
            }}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="When ON, AetherDesk also detects testing releases (tdesk-*/tdll-*) and gives them priority. Test updates are shown with a red dot. Keep OFF unless you are testing a build."
      >
        <span className="settings-toggle-text">Enable test updates</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={enableTestUpdates}
            onChange={(e) => setEnableTestUpdates(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="Opt-in fallback to the OpenSteamTool pattern source (OpenSteam001/steam-monitor) for Steam build patterns. OFF by default: only MigoReleases and KoriaPolis are used. Applies immediately (no Save, no Steam restart); takes effect on the next pattern download."
      >
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
              } catch (err: any) {
                setUseOstSource(previous);
                showStatus(`Failed to set OST pattern source: ${err}`, 'error');
              }
            }}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="When ON (default), AetherCore copies every backed-up .manifest (AetherData\backup\<app_id>\lua) that is missing from Steam\depotcache back into place on each Steam start. Uninstalling a game wipes its manifests, and Steam no longer serves manifests without authentication — this keeps them always available. Applies immediately (no Save); takes effect on the next Steam start."
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
              } catch (err: any) {
                setManifestRestoreOnStartup(previous);
                showStatus(`Failed to set manifest restore: ${err}`, 'error');
              }
            }}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="Controls the [presence] default_mode in aethercore.toml and applies immediately (no Save, no Steam restart). ON: friends see what you play for EVERY game you launch — unless the game has a per-game mode in the ONLINE popup (Show / Online Aether / excluded). OFF: nothing is shown by default; you hand-pick games per-game. With a Custom game display name set, every game is treated as showonline regardless of this policy."
      >
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
                await invoke('set_presence_default_mode', { showonline: next });
              } catch (err: any) {
                setPresenceDefaultShowOnline(previous);
                showStatus(`Failed to set presence default mode: ${err}`, 'error');
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
          placeholder="Enter custom game name (e.g. Aether)"
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
          <p className={`settings-desc steam-path-status steam-path-${steamCheck.state}`}>
            {steamCheck.message}
          </p>
        )}
      </div>

      <div className="settings-toggle-row" title="Choose the Steam folder with the system file picker">
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

      <div
        className="settings-toggle-row"
        title="Auto Detect the Steam installation (registry / running Steam)"
      >
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

      <div
        className="settings-toggle-row"
        title="Clear AetherDesk cache files such as store search, game info, Steam names and Denuvo cache. Settings and backups are preserved."
      >
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
              } catch {}
              showStatus(result, 'success');
            } catch (err: any) {
              showStatus(`Failed to clear caches: ${err}`, 'error');
            }
          }}
        >
          Clear Cache
        </button>
      </div>
    </div>
  );
};
