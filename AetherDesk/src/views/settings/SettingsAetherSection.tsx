import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { clearFrontendCaches } from '../../ui/cacheReset';
import { toErrorMessage } from '../../util/errors';

interface SettingsAetherSectionProps {
  enableWebviewDevtools: boolean;
  setEnableWebviewDevtools: (v: boolean) => void;
  enableTestUpdates: boolean;
  setEnableTestUpdates: (v: boolean) => void;
  manifestRestoreOnStartup: boolean;
  setManifestRestoreOnStartup: (v: boolean) => void;
  downloadGamesWithUpdatesOn: boolean;
  onDownloadGamesWithUpdatesChange: (checked: boolean) => void;
  showStatus: (message: string, kind: 'success' | 'error') => void;
}

export const SettingsAetherSection: React.FC<SettingsAetherSectionProps> = ({
  enableWebviewDevtools,
  setEnableWebviewDevtools,
  enableTestUpdates,
  setEnableTestUpdates,
  manifestRestoreOnStartup,
  setManifestRestoreOnStartup,
  downloadGamesWithUpdatesOn,
  onDownloadGamesWithUpdatesChange,
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

      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Clear AetherDesk caches</span>
        <button
          type="button"
          className="settings-small-btn settings-action-btn"
          onClick={async () => {
            try {
              const result: string = await invoke('clear_app_caches');
              clearFrontendCaches();
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
