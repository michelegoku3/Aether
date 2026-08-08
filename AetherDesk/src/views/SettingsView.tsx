import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface SettingsViewProps {
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
  onRefreshCustomCss: () => Promise<void>;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
}

export const SettingsView = ({ hubcapUsage, onRefreshUsage, onRefreshCustomCss, onPreviewPersonalWallpaper }: SettingsViewProps) => {
  const [apiKey, setApiKey] = useState('');
  const [steamPath, setSteamPath] = useState('C:\\Program Files (x86)\\Steam');
  const [activeLibrary, setActiveLibrary] = useState('');
  const [showStoreDlcs, setShowStoreDlcs] = useState(false);
  const [showStoreNsfw, setShowStoreNsfw] = useState(true);
  const [showStoreDelisted, setShowStoreDelisted] = useState(true);
  const [downloadGamesWithUpdatesOn, setDownloadGamesWithUpdatesOn] = useState(true);
  const [customCssEnabled, setCustomCssEnabled] = useState(false);
  const [personalWallpaperEnabled, setPersonalWallpaperEnabled] = useState(false);
  const [personalWallpaperOpacity, setPersonalWallpaperOpacity] = useState(35);
  const [ryuuKey, setRyuuKey] = useState('');
  const [storeCurrency, setStoreCurrency] = useState<'eur' | 'usd' | 'jpy'>('eur');

  // Raw settings as loaded from the backend. Saving always spreads this object
  // back, so fields owned by other flows (e.g. `antivirus_exclusion_done`) are
  // never silently reset by a settings save.
  const [rawSettings, setRawSettings] = useState<Record<string, any>>({});

  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' });

  // Load settings from the backend when the component mounts
  useEffect(() => {
    const loadSettings = async () => {
      try {
        const settings: any = await invoke('get_settings');
        if (settings) {
          setRawSettings(settings);
          setApiKey(settings.hubcap_api_key || '');
          setSteamPath(settings.steam_path || 'C:\\Program Files (x86)\\Steam');
          setActiveLibrary(settings.active_library || '');
          setShowStoreDlcs(Boolean(settings.show_store_dlcs));
          // These two default to enabled: only an explicit `false` turns them off.
          setShowStoreNsfw(settings.show_store_nsfw !== false);
          setShowStoreDelisted(settings.show_store_delisted !== false);
          setDownloadGamesWithUpdatesOn(settings.download_games_with_updates_on !== false);
          setCustomCssEnabled(Boolean(settings.custom_css_enabled));
          setPersonalWallpaperEnabled(Boolean(settings.personal_wallpaper_enabled));
          setPersonalWallpaperOpacity(Math.max(0, Math.min(100, Number(settings.personal_wallpaper_opacity ?? 35))));
          setRyuuKey(settings.ryuu_api_key || '');
          setStoreCurrency(['usd', 'jpy'].includes(settings.store_currency) ? settings.store_currency : 'eur');
        }
      } catch (err: any) {
        showStatus(`Error loading settings: ${err}`, 'error');
      }
    };
    loadSettings();
    onRefreshUsage();
  }, []);

  const showStatus = (text: string, type: 'info' | 'success' | 'error') => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    
    // Validazione API key se presente
    if (apiKey.trim()) {
      showStatus('Validating API key...', 'info');
      try {
        const isValid: any = await invoke('validate_hubcap_key', { apiKey });
        if (!isValid) {
          showStatus('Invalid API key. Settings not saved.', 'error');
          return;
        }
      } catch (err: any) {
        showStatus(`API key validation failed: ${err}`, 'error');
        return;
      }
    }
    
    // Salvataggio
    try {
      showStatus('Saving settings...', 'info');
      // Se l'utente ha appena attivato Custom CSS, assicurati che il file esista
      // prima di salvare, così la prossima apertura dell'editor non trova una cartella vuota.
      if (customCssEnabled || personalWallpaperEnabled) {
        try { await invoke('ensure_custom_css'); } catch {}
      }
      await invoke('save_settings', {
        settings: {
          ...rawSettings,
          hubcap_api_key: apiKey,
          steam_path: steamPath,
          active_library: activeLibrary,
          show_store_dlcs: showStoreDlcs,
          show_store_nsfw: showStoreNsfw,
          show_store_delisted: showStoreDelisted,
          custom_css_enabled: customCssEnabled,
          personal_wallpaper_enabled: personalWallpaperEnabled,
          personal_wallpaper_opacity: personalWallpaperOpacity,
          ryuu_api_key: ryuuKey,
          download_games_with_updates_on: downloadGamesWithUpdatesOn,
          store_currency: storeCurrency
        }
      });
      showStatus('Settings saved successfully!', 'success');
      onRefreshUsage(apiKey);
      onRefreshCustomCss();
    } catch (err: any) {
      showStatus(`Error during save: ${err}`, 'error');
    }
  };

  return (
    <div className="settings-view">
      <div className="settings-header">
        <h1 className="settings-title">Settings</h1>
        <p className="settings-subtitle">Manage system configurations, API keys, Steam injection paths, and other settings.</p>
      </div>

      <div className="settings-separator"></div>

      {statusMsg.text && (
        <div className={`settings-alert ${statusMsg.type}`}>
          {statusMsg.text}
        </div>
      )}

      <form onSubmit={handleSave} className="settings-form">
        <div className="settings-top-action-row" title="Clear AetherDesk cache files such as store search, game info, Steam names and Denuvo cache. Settings and backups are preserved.">
          <span className="settings-toggle-text">Clear AetherDesk caches</span>
          <button
            type="button"
            className="settings-small-btn"
            onClick={async () => {
              try {
                const result: string = await invoke('clear_app_caches');
                try {
                  Object.keys(localStorage)
                    .filter((key) => key.startsWith('aether_cover_'))
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

        <div className="settings-separator"></div>

        {/* Hubcap API Key Section */}
        <div className="settings-group">
          <label className="settings-label">Hubcap API Key</label>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
            <p className="settings-desc">Enter your hubcapmanifest.com API key to unlock database lookups and downloads.</p>
            {hubcapUsage.hasKey && (
              <span style={{ fontSize: '12px', color: '#8f8f9e', fontWeight: 'bold', marginLeft: '12px', whiteSpace: 'nowrap' }}>
                {hubcapUsage.usage}/{hubcapUsage.limit}
              </span>
            )}
          </div>
          <input 
            type="password" 
            placeholder="Enter API key (e.g. smm_...)"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            className="settings-input"
          />
        </div>

        {/* Ryuu API Key Section */}
        <div className="settings-group">
          <label className="settings-label">Ryuu API Key</label>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
            <p className="settings-desc">Enter your generator.ryuu.lol API key to unlock downloads via Ryuu.</p>
            {ryuuKey.trim() !== '' && (
              <span style={{ fontSize: '12px', color: '#8f8f9e', fontWeight: 'bold', marginLeft: '12px', whiteSpace: 'nowrap' }}>
                50/day
              </span>
            )}
          </div>
          <input 
            type="password" 
            placeholder="Enter Ryuu key (e.g. V1nr...)"
            value={ryuuKey}
            onChange={(e) => setRyuuKey(e.target.value)}
            className="settings-input"
          />
        </div>

        <div className="settings-separator"></div>

        {/* Steam Path Section */}
        <div className="settings-group">
          <label className="settings-label">Steam Installation Path</label>
          <p className="settings-desc">The main directory path where Steam is installed on your PC, required for configuration.</p>
          <input 
            type="text" 
            placeholder="C:\Program Files (x86)\Steam"
            value={steamPath}
            onChange={(e) => setSteamPath(e.target.value)}
            className="settings-input"
          />
        </div>

        <div className="settings-separator"></div>

        {/* Store Section */}
        <div className="settings-group">
          <label className="settings-label">Store</label>
          <div className="settings-toggle-row" title="Show downloadable and non-downloadable add-ons (DLC) in store search results">
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

          <div className="settings-toggle-row" title="Show delisted games (removed from the Steam catalog) in store search results; they are highlighted with a white border">
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

          <div className="settings-toggle-row" title="Show adult-only (NSFW) games in store search results; they are highlighted with a pink border">
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

          <div className="settings-toggle-row" title="After latest-version downloads, comment setManifestid pins so Steam can update the game normally">
            <span className="settings-toggle-text">Download games with updates on</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={downloadGamesWithUpdatesOn}
                onChange={(e) => setDownloadGamesWithUpdatesOn(e.target.checked)}
              />
              <span></span>
            </label>
          </div>

          <div className="settings-toggle-row" title="Preferred currency for Steam prices shown in Store and Info">
            <span className="settings-toggle-text">Store price currency</span>
            <select
              className="settings-select"
              value={storeCurrency}
              onChange={(e) => setStoreCurrency(e.target.value as 'eur' | 'usd' | 'jpy')}
            >
              <option value="eur">Euro (€)</option>
              <option value="usd">Dollar ($)</option>
              <option value="jpy">Yen (¥)</option>
            </select>
          </div>
        </div>

        <div className="settings-separator"></div>

        {/* Appearance — Custom CSS */}
        <div className="settings-group">
          <label className="settings-label">Appearance</label>
          <div className="settings-toggle-row" title="Load AetherData/config/custom.css after the default theme.">
            <span className="settings-toggle-text">Enable Custom CSS</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={customCssEnabled}
                onChange={async (e) => {
                  const next = e.target.checked;
                  setCustomCssEnabled(next);
                  if (next) {
                    try { await invoke('ensure_custom_css'); } catch {}
                  }
                }}
              />
              <span></span>
            </label>
          </div>

          <div className="settings-toggle-row" title="Use AetherData/config/wallpaper.<ext> as the app background.">
            <span className="settings-toggle-text">Enable personal wallpaper</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={personalWallpaperEnabled}
                onChange={async (e) => {
                  const next = e.target.checked;
                  setPersonalWallpaperEnabled(next);
                  onPreviewPersonalWallpaper(next, personalWallpaperOpacity);
                  if (next) {
                    try { await invoke('ensure_custom_css'); } catch {}
                  }
                }}
              />
              <span></span>
            </label>
          </div>

          {personalWallpaperEnabled && (
            <div className="settings-toggle-row" title="Adjust the personal wallpaper opacity from 0 to 100.">
              <span className="settings-toggle-text">Wallpaper opacity</span>
              <input
                type="text"
                inputMode="numeric"
                pattern="[0-9]*"
                className="settings-number-input"
                value={personalWallpaperOpacity}
                onChange={(e) => {
                  const numeric = e.target.value.replace(/\D/g, '');
                  const next = Math.max(0, Math.min(100, Number(numeric || 0)));
                  setPersonalWallpaperOpacity(next);
                  onPreviewPersonalWallpaper(personalWallpaperEnabled, next);
                }}
              />
            </div>
          )}

        </div>

        <div className="settings-separator"></div>

        <div className="form-actions" style={{ justifyContent: 'center', gap: '12px' }}>
          <button type="submit" className="save-settings-btn" style={{ flex: '1 1 0', maxWidth: '200px' }}>
            Save Settings
          </button>
          <button
            type="button"
            className="save-settings-btn"
            style={{ flex: '1 1 0', maxWidth: '200px', backgroundColor: '#1c1c21', border: '1px solid var(--border-color)' }}
            onClick={async () => {
              // Reset to defaults (matching Rust AppSettings::default)
              setApiKey('');
              setRyuuKey('');
              setSteamPath('C:\\Program Files (x86)\\Steam');
              setActiveLibrary('');
              setShowStoreDlcs(false);
              setShowStoreNsfw(true);
              setShowStoreDelisted(true);
              setDownloadGamesWithUpdatesOn(true);
              setCustomCssEnabled(false);
              setPersonalWallpaperEnabled(false);
              setPersonalWallpaperOpacity(35);
              onPreviewPersonalWallpaper(false, 35);
              setStoreCurrency('eur');
              try {
                await invoke('save_settings', {
                  settings: {
                    ...rawSettings,
                    hubcap_api_key: '',
                    ryuu_api_key: '',
                    steam_path: 'C:\\Program Files (x86)\\Steam',
                    active_library: '',
                    show_store_dlcs: false,
                    show_store_nsfw: true,
                    show_store_delisted: true,
                    download_games_with_updates_on: true,
                    custom_css_enabled: false,
                    personal_wallpaper_enabled: false,
                    personal_wallpaper_opacity: 35,
                    store_currency: 'eur',
                    antivirus_exclusion_done: rawSettings.antivirus_exclusion_done ?? false
                  }
                });
                showStatus('Settings reset to defaults!', 'success');
                onRefreshUsage('');
                onRefreshCustomCss();
              } catch (err: any) {
                showStatus(`Failed to reset settings: ${err}`, 'error');
              }
            }}
          >
            Reset Settings
          </button>
        </div>
      </form>
    </div>
  );
};
