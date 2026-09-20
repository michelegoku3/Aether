import React from 'react';
import { STORE_CURRENCIES, type StoreCurrency } from '../../hooks/useSettings';

interface SettingsStoreSectionProps {
  showStoreDlcs: boolean;
  setShowStoreDlcs: (v: boolean) => void;
  showStoreDelisted: boolean;
  setShowStoreDelisted: (v: boolean) => void;
  showStoreNsfw: boolean;
  setShowStoreNsfw: (v: boolean) => void;
  showStoreFrontGames: boolean;
  setShowStoreFrontGames: (v: boolean) => void;
  storeFrontFilter: string;
  setStoreFrontFilter: (v: string) => void;
  storeCurrency: StoreCurrency;
  setStoreCurrency: (v: StoreCurrency) => void;
  downloadGamesWithUpdatesOn: boolean;
  onDownloadGamesWithUpdatesChange: (checked: boolean) => void;
  workshopAutoDownloadContent: boolean;
  setWorkshopAutoDownloadContent: (v: boolean) => void;
}

export const SettingsStoreSection: React.FC<SettingsStoreSectionProps> = ({
  showStoreDlcs,
  setShowStoreDlcs,
  showStoreDelisted,
  setShowStoreDelisted,
  showStoreNsfw,
  setShowStoreNsfw,
  showStoreFrontGames,
  setShowStoreFrontGames,
  storeFrontFilter,
  setStoreFrontFilter,
  storeCurrency,
  setStoreCurrency,
  downloadGamesWithUpdatesOn,
  onDownloadGamesWithUpdatesChange,
  workshopAutoDownloadContent,
  setWorkshopAutoDownloadContent,
}) => {
  return (
    <div className="settings-group">
      <label className="settings-label">Store</label>
      <div
        className="settings-toggle-row"
        title="Show downloadable and non-downloadable add-ons (DLC) in store search results"
      >
        <span className="settings-toggle-text">Show DLCs in the store</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={showStoreDlcs}
            onChange={(e) => setShowStoreDlcs(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="Show delisted games (removed from the Steam catalog) in store search results; they are highlighted with a white border"
      >
        <span className="settings-toggle-text">Show delisted games in the store</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={showStoreDelisted}
            onChange={(e) => setShowStoreDelisted(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="Show adult-only (NSFW) games in store search results; they are highlighted with a pink border"
      >
        <span className="settings-toggle-text">Show NSFW games in the store</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={showStoreNsfw}
            onChange={(e) => setShowStoreNsfw(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      <div
        className="settings-toggle-row"
        title="Show a Steam Store front page when no search query is active"
      >
        <span className="settings-toggle-text">Show store front games</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={showStoreFrontGames}
            onChange={(e) => setShowStoreFrontGames(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      {showStoreFrontGames && (
        <div
          className="settings-toggle-row"
          title="Choose which Steam Store front criterion is shown by default"
        >
          <span className="settings-toggle-text">Store front criterion</span>
          <select
            className="settings-front-filter-select"
            value={storeFrontFilter}
            onChange={(e) => setStoreFrontFilter(e.target.value)}
          >
            <option value="trending">Trending</option>
            <option value="latest">Latest</option>
            <option value="top_sellers">Top sellers</option>
            <option value="upcoming">Upcoming</option>
            <option value="discounts">Discounts</option>
          </select>
        </div>
      )}

      <div
        className="settings-toggle-row"
        title="Preferred currency for Steam prices shown in Store and Info"
      >
        <span className="settings-toggle-text">Store price currency</span>
        <select
          className="settings-select"
          value={storeCurrency}
          onChange={(e) => setStoreCurrency(e.target.value as StoreCurrency)}
        >
          {STORE_CURRENCIES.map(({ code, label }) => (
            <option key={code} value={code}>
              {label}
            </option>
          ))}
        </select>
      </div>

      <div
        className="settings-toggle-row"
        title="After latest-version downloads, comment setManifestid pins so Steam can update the game normally. Requires a valid active Hubcap key."
      >
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
        title="When Workshop items have a staged manifest but no content, ask Steam to download them one at a time (spaced). Turn off to stage manifests only — content then downloads on demand. Prevents a download storm after a depotcache wipe."
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
    </div>
  );
};
