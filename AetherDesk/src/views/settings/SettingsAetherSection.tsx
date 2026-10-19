import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toErrorMessage } from '../../util/errors';

interface SettingsAetherSectionProps {
  enableWebviewDevtools: boolean;
  setEnableWebviewDevtools: (v: boolean) => void;
  enableTestUpdates: boolean;
  setEnableTestUpdates: (v: boolean) => void;
  useOstSource: boolean;
  setUseOstSource: (v: boolean) => void;
  manifestRestoreOnStartup: boolean;
  setManifestRestoreOnStartup: (v: boolean) => void;
  downloadGamesWithUpdatesOn: boolean;
  onDownloadGamesWithUpdatesChange: (checked: boolean) => void;
  workshopAutoDownloadContent: boolean;
  setWorkshopAutoDownloadContent: (v: boolean) => void;
  ostWarningAcknowledged: boolean;
  setShowOstWarning: (v: boolean) => void;
  showStatus: (message: string, kind: 'success' | 'error') => void;
}

export const SettingsAetherSection: React.FC<SettingsAetherSectionProps> = ({
  enableWebviewDevtools,
  setEnableWebviewDevtools,
  enableTestUpdates,
  setEnableTestUpdates,
  useOstSource,
  setUseOstSource,
  manifestRestoreOnStartup,
  setManifestRestoreOnStartup,
  downloadGamesWithUpdatesOn,
  onDownloadGamesWithUpdatesChange,
  workshopAutoDownloadContent,
  setWorkshopAutoDownloadContent,
  ostWarningAcknowledged,
  setShowOstWarning,
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
        <span className="settings-toggle-text">Download games with updates on</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={downloadGamesWithUpdatesOn}
            onChange={(e) => void onDownloadGamesWithUpdatesChange(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="When Workshop items have a staged manifest but no content, ask Steam to download them one at a time, preventing a download storm after a depotcache wipe."
      >
        <span className="settings-toggle-text">Workshop auto-download content</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={workshopAutoDownloadContent}
            onChange={(e) => setWorkshopAutoDownloadContent(e.target.checked)}
          />
          <span></span>
        </label>
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
