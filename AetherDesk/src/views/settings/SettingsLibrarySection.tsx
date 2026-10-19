import React from 'react';
import { parseAppIdList, type SteamAccount } from './libraryBanner';

interface SettingsLibrarySectionProps {
  enabled: boolean;
  setEnabled: (value: boolean) => void;
  appIds: string;
  setAppIds: (value: string) => void;
  steamId: string;
  setSteamId: (value: string) => void;
  accounts: SteamAccount[];
  accountsLoaded: boolean;
  /** Reading loginusers.vdf failed: distinct from "Steam has no accounts". */
  accountsError: boolean;
}

/**
 * Cosmetic library-owner banner controls, rendered inside the Steam settings
 * group (no title of its own).
 *
 * Purely visual: AetherCore stamps the chosen account as `LastOwner` on the
 * listed App IDs so Steam renders "From the library of ...", while every other
 * managed game keeps the entry removed. Nothing about ownership, licenses or
 * downloads changes.
 */
export const SettingsLibrarySection: React.FC<SettingsLibrarySectionProps> = ({
  enabled,
  setEnabled,
  appIds,
  setAppIds,
  steamId,
  setSteamId,
  accounts,
  accountsLoaded,
  accountsError,
}) => {
  const { invalid } = parseAppIdList(appIds);
  const knownSelection = accounts.some((account) => account.steamId === steamId);

  return (
    <>
      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Show &quot;From the library of ...&quot; on selected games</span>
        <label className="version-switch">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
          <span></span>
        </label>
      </div>

      {enabled && (
        <>
          <div className="settings-toggle-row settings-field-column">
            <span className="settings-toggle-text">App IDs</span>
            <input
              type="text"
              className="settings-input"
              placeholder="e.g. 4656000, 4108000, 3618390"
              value={appIds}
              onChange={(e) => setAppIds(e.target.value)}
            />
            {invalid.length > 0 && (
              <p className="settings-error">Invalid App ID: {invalid.map((token) => `"${token}"`).join(', ')}</p>
            )}
          </div>

          <div className="settings-toggle-row settings-field-column">
            <span className="settings-toggle-text">Library owner</span>
            <select
              className="settings-input settings-input--select"
              value={steamId}
              onChange={(e) => setSteamId(e.target.value)}
            >
              <option value="">Select an account...</option>
              {steamId && !knownSelection && <option value={steamId}>{steamId}</option>}
              {accounts.map((account) => (
                <option key={account.steamId} value={account.steamId}>
                  {account.personaName || account.accountName} ({account.steamId})
                </option>
              ))}
            </select>
            {accountsError ? (
              <p className="settings-error">
                Could not read the local Steam accounts. Check the Steam installation path.
              </p>
            ) : (
              accountsLoaded &&
              accounts.length === 0 && (
                <p className="settings-desc">
                  No local Steam accounts found. Sign in to Steam once with the account you want to show.
                </p>
              )
            )}
            <p className="settings-note">
              Selected games will be shown as games from another library. A Steam restart is required.
            </p>
          </div>
        </>
      )}
    </>
  );
};
