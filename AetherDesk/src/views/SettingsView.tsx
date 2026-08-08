import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface SettingsViewProps {
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
  onRefreshCustomCss: () => Promise<void>;
}

export const SettingsView = ({ hubcapUsage, onRefreshUsage, onRefreshCustomCss }: SettingsViewProps) => {
  const [apiKey, setApiKey] = useState('');
  const [steamPath, setSteamPath] = useState('C:\\Program Files (x86)\\Steam');
  const [activeLibrary, setActiveLibrary] = useState('');
  const [showStoreDlcs, setShowStoreDlcs] = useState(false);
  const [showStoreNsfw, setShowStoreNsfw] = useState(true);
  const [showStoreDelisted, setShowStoreDelisted] = useState(true);
  const [customCssEnabled, setCustomCssEnabled] = useState(false);
  const [ryuuKey, setRyuuKey] = useState('');

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
          setCustomCssEnabled(Boolean(settings.custom_css_enabled));
          setRyuuKey(settings.ryuu_api_key || '');
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
      if (customCssEnabled) {
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
          ryuu_api_key: ryuuKey
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
              setCustomCssEnabled(false);
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
                    custom_css_enabled: false,
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
